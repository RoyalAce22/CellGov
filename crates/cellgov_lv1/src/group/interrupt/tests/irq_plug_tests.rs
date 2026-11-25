use super::*;
use crate::group::spe::LogicalSpe;
use crate::IrqStateBitmapRegistration;
use crate::{AddressSpace, AddressSpaceChange};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::CONNECT_IRQ_PLUG_EXT;
use cellgov_time::GuestTicks;

/// The logical PPE id of the 4.93 GameOS partition, `bi.pu` at PU index 0.
const PPE_ID: u64 = 1;

/// The port id of the 4.93 kernel's first port: +0x40 of the record of
/// thread 0 in the capture `lv2_logical_ppe_record`.
const FIRST_PORT_ID: u64 = 3;

/// The vas id that the 4.93 kernel's boot `construct_virtual_address_space`
/// gets.
const VAS_ID: u64 = 2;

/// The spe id of the 4.93 kernel's boot SPE in the capture
/// `lv2_logical_spe_object`.
const BOOT_SPE_ID: u64 = 11;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

fn thread(thread: u8) -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: PPE_ID,
        thread,
    }
}

/// The 4.93 kernel's call: `sc 1` [OFW-4.93 lv2_kernel a:0x80000000000087f8],
/// with the logical PPE id and thread 0 from the thread's record, plug 0,
/// the first port and r7 = 0.
fn request() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[PPE_ID, 0, 0, FIRST_PORT_ID, 0]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONNECT_IRQ_PLUG_EXT,
        inputs,
        pc: 0x8000_0000_0000_87f8,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(52_119),
    }
}

fn with(r: &HypercallRequest, reg: usize, value: u64) -> HypercallRequest {
    let mut changed = *r;
    changed.inputs[reg - 3] = value;
    changed
}

/// The hypervisor's state when the kernel's first connect step runs on
/// thread 0: the vas, the bitmap of thread 0 and the ports `ports`.
struct Held {
    spaces: AddressSpaceState,
    spes: LogicalSpeState,
    bitmaps: IrqStateBitmapState,
    ports: EventReceivePortState,
    storage: GuestMemory,
}

impl Held {
    fn boot(ports: &[EventReceivePort]) -> Self {
        let mut spaces = AddressSpaceState::default();
        spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0,
            mapping: None,
        }));
        let mut bitmaps = IrqStateBitmapState::default();
        bitmaps.register(IrqStateBitmapRegistration {
            key: thread(0),
            lpar_address: THREAD_0_BITMAP,
        });
        let mut state = EventReceivePortState::default();
        for &port in ports {
            state.construct(port);
        }
        Self {
            spaces,
            spes: LogicalSpeState::default(),
            bitmaps,
            ports: state,
            storage: GuestMemory::new(0x0037_0000),
        }
    }

    fn connect(&self, r: &HypercallRequest) -> HypercallDisposition {
        self.connect_at(r, Some("4.93"))
    }

    fn connect_at(&self, r: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
        connect_plug(
            r,
            firmware,
            &PlugState {
                spaces: &self.spaces,
                spes: &self.spes,
                bitmaps: &self.bitmaps,
                ports: &self.ports,
                storage: &self.storage,
            },
        )
    }
}

fn first_port() -> Held {
    Held::boot(&[EventReceivePort::new(FIRST_PORT_ID)])
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

/// The answer that connects `port` to `plug` of `key`, with a write of 0
/// to the status doubleword at `status`, if any.
fn connected(
    r: &HypercallRequest,
    port: u64,
    key: IrqStateBitmapKey,
    plug: u8,
    status: Option<u64>,
) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    if let Some(address) = status {
        answer.effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(address), 8).unwrap(),
            bytes: WritePayload::new(vec![0; 8]),
            source: r.unit,
            source_time: r.tick,
        });
    }
    answer.push_change(InterruptChange::PlugConnect(EventReceivePort {
        id: port,
        connection: Some(key),
        plug,
    }));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONNECT_IRQ_PLUG_EXT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn the_kernels_first_call_connects_the_first_port_and_clears_the_plug_s_status_doubleword() {
    let r = request();
    assert_eq!(
        first_port().connect(&r),
        connected(&r, FIRST_PORT_ID, thread(0), 0, Some(THREAD_0_BITMAP))
    );
}

#[test]
fn a_plug_writes_the_status_doubleword_that_holds_its_bit() {
    for (plug, status) in [
        (63, THREAD_0_BITMAP),
        (64, THREAD_0_BITMAP + 8),
        (130, THREAD_0_BITMAP + 16),
        (255, THREAD_0_BITMAP + 24),
    ] {
        let r = with(&request(), 5, plug);
        assert_eq!(
            first_port().connect(&r),
            connected(&r, FIRST_PORT_ID, thread(0), plug as u8, Some(status)),
            "plug {plug}"
        );
    }
}

#[test]
fn a_thread_with_no_registered_bitmap_gets_no_write() {
    let r = with(&request(), 4, 1);
    assert_eq!(
        first_port().connect(&r),
        connected(&r, FIRST_PORT_ID, thread(1), 0, None)
    );
}

#[test]
fn the_answer_does_not_read_the_calling_thread() {
    let mut r = request();
    for calling in [Some(0), Some(1), None] {
        r.thread = calling;
        assert_eq!(
            first_port().connect(&r),
            connected(&r, FIRST_PORT_ID, thread(0), 0, Some(THREAD_0_BITMAP)),
            "{calling:?}"
        );
    }
}

#[test]
fn each_argument_check_gives_lv1_s_marked_r3() {
    let r = request();
    let held = first_port();
    for (reg, value, r3) in [
        (3, 2, IRQ_PLUG_PPE_ID_R3),
        (3, FIRST_PORT_ID, IRQ_PLUG_PPE_ID_R3),
        (4, 2, IRQ_PLUG_THREAD_R3),
        (4, 0x1_0000_0000, IRQ_PLUG_THREAD_R3),
        (5, 256, IRQ_PLUG_PLUG_R3),
        (5, u64::MAX, IRQ_PLUG_PLUG_R3),
        (7, 1, IRQ_PLUG_R7_R3),
        (6, PPE_ID, IRQ_PLUG_PORT_R3),
        (6, VAS_ID, IRQ_PLUG_PORT_R3),
    ] {
        assert_eq!(
            held.connect(&with(&r, reg, value)),
            lv1_r3(r3),
            "r{reg} = 0x{value:x}"
        );
    }
}

#[test]
fn the_checks_run_in_lv1_s_order() {
    let mut r = request();
    r.inputs[..5].copy_from_slice(&[2, 2, 256, PPE_ID, 1]);
    let held = first_port();
    let mut expected = Vec::new();
    for (reg, good) in [(3, PPE_ID), (4, 0), (5, 0), (7, 0), (6, FIRST_PORT_ID)] {
        expected.push(held.connect(&r));
        r = with(&r, reg, good);
    }
    assert_eq!(
        expected,
        [
            IRQ_PLUG_PPE_ID_R3,
            IRQ_PLUG_THREAD_R3,
            IRQ_PLUG_PLUG_R3,
            IRQ_PLUG_R7_R3,
            IRQ_PLUG_PORT_R3,
        ]
        .map(lv1_r3)
    );
}

#[test]
fn a_connected_port_gets_wrong_state_and_no_change() {
    let port = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread(1)),
        plug: 7,
    };
    assert_eq!(
        Held::boot(&[port]).connect(&request()),
        lv1_r3(IRQ_PLUG_PORT_CONNECTED_R3)
    );
}

#[test]
fn a_repeat_of_the_connected_call_gets_wrong_state_before_the_plug_check() {
    let port = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread(0)),
        plug: 0,
    };
    assert_eq!(
        Held::boot(&[port]).connect(&request()),
        lv1_r3(IRQ_PLUG_PORT_CONNECTED_R3)
    );
}

#[test]
fn a_plug_that_holds_a_port_leaves_the_plug_in_the_new_port_unconnected() {
    let first = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread(0)),
        plug: 5,
    };
    let held = Held::boot(&[first, EventReceivePort::new(4)]);
    let r = with(&with(&request(), 6, 4), 5, 5);
    let mut answer = HypercallAnswer::lv1_status(IRQ_PLUG_IN_USE_R3, [0; 7]);
    answer.push_change(InterruptChange::PlugConnect(EventReceivePort {
        id: 4,
        connection: None,
        plug: 5,
    }));
    assert_eq!(held.connect(&r), HypercallDisposition::Answered(answer));
}

#[test]
fn the_same_plug_of_the_other_thread_is_free() {
    let first = EventReceivePort {
        id: FIRST_PORT_ID,
        connection: Some(thread(1)),
        plug: 0,
    };
    let held = Held::boot(&[first, EventReceivePort::new(4)]);
    let r = with(&request(), 6, 4);
    assert_eq!(
        held.connect(&r),
        connected(&r, 4, thread(0), 0, Some(THREAD_0_BITMAP))
    );
}

#[test]
fn a_port_id_that_names_no_held_object_is_refused() {
    let r = request();
    for port_id in [0, 4, 11, u64::MAX] {
        let r = with(&r, 6, port_id);
        assert_eq!(
            first_port().connect(&r),
            refused(
                &r,
                "an IRQ plug call whose port id names no object CellGov holds"
            ),
            "{port_id}"
        );
    }
}

/// lv1 reads the SPE class at +48 of a logical SPE, and refuses it as a
/// port [OFW-4.93 lv1 a:0x2EA18C-0x2EA198].
#[test]
fn a_logical_spe_id_gets_lv1_s_port_r3_and_no_change() {
    let r = with(&request(), 6, BOOT_SPE_ID);
    let mut held = first_port();
    held.spes.construct(LogicalSpe {
        id: BOOT_SPE_ID,
        spe_type: 1,
        vas_id: VAS_ID,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    });
    assert_eq!(held.connect(&r), lv1_r3(IRQ_PLUG_PORT_R3));
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_refused() {
    let r = request();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            first_port().connect_at(&r, firmware),
            refused(
                &r,
                "an IRQ plug call at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

#[test]
fn a_status_doubleword_with_no_storage_is_refused() {
    let r = request();
    let mut held = first_port();
    held.storage = GuestMemory::new(THREAD_0_BITMAP as usize);
    assert_eq!(
        held.connect(&r),
        refused(
            &r,
            "an IRQ plug whose status doubleword CellGov holds no storage for"
        )
    );
}
