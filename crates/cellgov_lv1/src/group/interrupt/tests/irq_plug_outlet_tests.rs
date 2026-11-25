//! Hypercall 12 connects an irq outlet of a logical SPE as it connects an
//! event receive port, and a plug holds one port or one outlet.

use super::*;
use crate::IrqStateBitmapRegistration;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONNECT_IRQ_PLUG_EXT;
use cellgov_ps3_abi::lv1::spe::game_os_spe_call;
use cellgov_time::GuestTicks;

/// The logical PPE id of the 4.93 GameOS partition, `bi.pu` at PU index 0.
const PPE_ID: u64 = 1;

/// The port id of the 4.93 kernel's first port: +0x40 of the record of
/// thread 0 in the capture `lv2_logical_ppe_record`.
const FIRST_PORT_ID: u64 = 3;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The class 0 outlet of the 4.93 kernel's boot SPE, which the capture
/// `lv2_logical_spe_object` reads.
fn outlet_0() -> u64 {
    game_os_spe_call("4.93", 0x8000_0000_0004_f188)
        .expect("the boot SPE's ids")
        .outlets[0]
}

fn thread_0() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: PPE_ID,
        thread: 0,
    }
}

/// The 4.93 kernel's call on thread 0 [OFW-4.93 lv2_kernel a:0x80000000000087f8]
/// with the plug `plug` and the id `id` in r6.
fn request(plug: u64, id: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[PPE_ID, 0, plug, id, 0]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONNECT_IRQ_PLUG_EXT,
        inputs,
        pc: 0x8000_0000_0000_87f8,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_304),
    }
}

/// The hypervisor's state for a connect step on thread 0: the bitmap of
/// thread 0, the ports `ports` and the outlets `outlets`.
fn connect(
    r: &HypercallRequest,
    ports: &[EventReceivePort],
    outlets: &[EventReceivePort],
) -> HypercallDisposition {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.register(IrqStateBitmapRegistration {
        key: thread_0(),
        lpar_address: THREAD_0_BITMAP,
    });
    let mut held = EventReceivePortState::default();
    for &port in ports {
        held.construct(port);
    }
    for &outlet in outlets {
        held.construct_outlet(outlet);
    }
    connect_plug(
        r,
        Some("4.93"),
        &PlugState {
            spaces: &AddressSpaceState::default(),
            bitmaps: &bitmaps,
            ports: &held,
            storage: &GuestMemory::new(0x0037_0000),
        },
    )
}

/// The answer with lv1's r3 `r3`, r4 to r10 cleared, the status
/// doubleword write `status`, if any, and the change `change`.
fn answer(
    r: &HypercallRequest,
    r3: u64,
    status: Option<u64>,
    change: InterruptChange,
) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(r3, [0; 7]);
    if let Some(address) = status {
        answer.effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(address), 8).expect("a range"),
            bytes: WritePayload::new(vec![0; 8]),
            source: r.unit,
            source_time: r.tick,
        });
    }
    answer.push_change(change);
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_kernels_call_connects_the_boot_spe_s_class_0_outlet() {
    let r = request(4, outlet_0());
    assert_eq!(
        connect(&r, &[], &[EventReceivePort::new(outlet_0())]),
        answer(
            &r,
            0,
            Some(THREAD_0_BITMAP),
            InterruptChange::OutletPlugConnect(EventReceivePort {
                id: outlet_0(),
                connection: Some(thread_0()),
                plug: 4,
            })
        )
    );
}

#[test]
fn a_connected_outlet_gets_wrong_state_and_no_change() {
    let outlet = EventReceivePort {
        id: outlet_0(),
        connection: Some(thread_0()),
        plug: 4,
    };
    assert_eq!(
        connect(&request(5, outlet_0()), &[], &[outlet]),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            IRQ_PLUG_PORT_CONNECTED_R3,
            [0; 7]
        ))
    );
}

#[test]
fn a_plug_that_an_outlet_holds_is_in_use_for_a_port() {
    let outlet = EventReceivePort {
        id: outlet_0(),
        connection: Some(thread_0()),
        plug: 5,
    };
    let r = request(5, FIRST_PORT_ID);
    assert_eq!(
        connect(&r, &[EventReceivePort::new(FIRST_PORT_ID)], &[outlet]),
        answer(
            &r,
            IRQ_PLUG_IN_USE_R3,
            None,
            InterruptChange::PlugConnect(EventReceivePort {
                id: FIRST_PORT_ID,
                connection: None,
                plug: 5,
            })
        )
    );
}

#[test]
fn a_plug_that_a_port_holds_is_in_use_for_an_outlet() {
    let port = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread_0()),
        plug: 5,
    };
    let r = request(5, outlet_0());
    assert_eq!(
        connect(&r, &[port], &[EventReceivePort::new(outlet_0())]),
        answer(
            &r,
            IRQ_PLUG_IN_USE_R3,
            None,
            InterruptChange::OutletPlugConnect(EventReceivePort {
                id: outlet_0(),
                connection: None,
                plug: 5,
            })
        )
    );
}
