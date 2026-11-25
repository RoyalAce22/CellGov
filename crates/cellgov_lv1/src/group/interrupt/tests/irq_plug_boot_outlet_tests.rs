//! The hypervisor answers the four connect steps of the 4.93 kernel's boot
//! loop after the construct of its boot SPE: the SPE's outlets of classes
//! 0 to 2, then the port at +88 of the kernel's SPE object
//! [OFW-4.93 lv2_kernel a:0x8000000000034aac-0x8000000000034ae8]
//! [OFW-4.93 lv2_kernel a:0x800000000004ee14-0x800000000004ee38].
//!
//! The capture `lv2_boot_spe_plug_connects` gives the plug of each step,
//! and shows that lv1 answered each step with r3 = 0 on the console.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    CONSTRUCT_LOGICAL_SPE, CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
};
use cellgov_ps3_abi::lv1::event_port::game_os_port_id;
use cellgov_ps3_abi::lv1::spe::game_os_spe_call;
use cellgov_time::GuestTicks;

/// The bitmap of the 4.93 kernel's thread 0: +0x1C0 of its record, which
/// the capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The `sc` of the kernel's boot construct of its SPE
/// [OFW-4.93 lv2_kernel a:0x800000000004f188].
const SPE_CONSTRUCT_SC: u64 = 0x8000_0000_0004_f188;

/// The `sc` of the port call in the kernel's create function of its boot
/// SPE [OFW-4.93 lv2_kernel a:0x800000000004f218].
const SPE_PORT_SC: u64 = 0x8000_0000_0004_f218;

fn storage() -> GuestMemory {
    GuestMemory::new(0x0037_0000)
}

/// A thread 0 call of hypercall `number` at the `sc` address `pc`.
fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The bytes of one plug entry in the record of a thread: the kernel's
/// connect step takes the entry at +48 of the record plus 88 * plug
/// [OFW-4.93 lv2_kernel a:0x800000000000880c-0x8000000000008814].
const PLUG_ENTRY_BYTES: u64 = 88;

/// The value of the field `field` in the capture. Both reads of the word
/// must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_boot_spe_plug_connects")
        .join("peek/cech20-cex-493/plug_connects.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// The plug of each step on the console: its entry's offset from the
/// plug entries of thread 0, in entries. The index word of each step's
/// object must name the step.
fn captured_plugs() -> [u8; 4] {
    let entries = captured("plug_entries");
    [0u64, 1, 2, 3].map(|step| {
        assert_eq!(
            captured(&format!("index_{step}")) >> 32,
            step,
            "step {step}"
        );
        let offset = captured(&format!("entry_{step}")) - entries;
        assert_eq!(offset % PLUG_ENTRY_BYTES, 0, "step {step}: entry offset");
        u8::try_from(offset / PLUG_ENTRY_BYTES).expect("a plug below 256")
    })
}

/// The ids that the four steps pass in r6: the boot SPE's outlets of
/// classes 0 to 2, then its port. The capture's SPE object must hold the
/// spe id of that SPE.
fn boot_loop_ids() -> [u64; 4] {
    let spe = game_os_spe_call("4.93", SPE_CONSTRUCT_SC).expect("the boot SPE's ids");
    assert_eq!(captured("spe_id"), spe.id, "the capture's SPE object");
    let [outlet_0, outlet_1, outlet_2] = spe.outlets;
    let port = game_os_port_id("4.93", 0, SPE_PORT_SC).expect("the boot SPE's port id");
    [outlet_0, outlet_1, outlet_2, port]
}

/// A 4.93 hypervisor after the kernel's calls that the boot loop needs:
/// the address space, the bitmap of thread 0, the boot SPE and its port.
fn booted() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in [
        call(
            CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
            0,
            &[0, 2, 0x1014_0000_0000_0000],
        ),
        call(CONFIGURE_IRQ_STATE_BITMAP, 0, &[1, 0, THREAD_0_BITMAP]),
        call(
            CONSTRUCT_LOGICAL_SPE,
            SPE_CONSTRUCT_SC,
            &[12, 12, 12, 12, 12, 2, 1],
        ),
        call(CONSTRUCT_EVENT_RECEIVE_PORT, SPE_PORT_SC, &[]),
    ] {
        assert!(
            matches!(
                hv.dispatch_with(&setup, &storage()),
                HypercallDisposition::Answered(_)
            ),
            "hypercall {}: {:?}",
            setup.number,
            hv.refusal()
        );
    }
    hv
}

fn thread_0() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: 1,
        thread: 0,
    }
}

#[test]
fn the_hypervisor_answers_the_four_steps_on_the_captured_plugs_and_keeps_and_hashes_each() {
    let mut hv = booted();
    let ids = boot_loop_ids();
    let plugs = captured_plugs();
    for (plug, id) in plugs.into_iter().zip(ids) {
        let before = hv.sync_term();
        let r = call(
            CONNECT_IRQ_PLUG_EXT,
            0x8000_0000_0000_87f8,
            &[1, 0, u64::from(plug), id, 0],
        );
        let HypercallDisposition::Answered(answer) = hv.dispatch_with(&r, &storage()) else {
            panic!("r6 = {id}: {:?}", hv.refusal());
        };
        assert_eq!(answer.r3, 0, "r6 = {id}");
        assert_eq!(answer.outputs(), Some([0; 7]), "r6 = {id}");
        assert_ne!(hv.sync_term(), before, "r6 = {id}");
    }
    let connected = |id, plug| EventReceivePort {
        id,
        connection: Some(thread_0()),
        plug,
    };
    let ports = hv.event_receive_ports();
    let outlets: Vec<EventReceivePort> = ports.outlets().values().copied().collect();
    assert_eq!(
        outlets,
        [
            connected(ids[0], plugs[0]),
            connected(ids[1], plugs[1]),
            connected(ids[2], plugs[2])
        ]
    );
    assert_eq!(ports.port(ids[3]), Some(&connected(ids[3], plugs[3])));
    for (step, (id, plug)) in ids.into_iter().zip(plugs).enumerate() {
        assert_eq!(
            ports.port_on_plug(thread_0(), plug),
            Some(&connected(id, plug)),
            "step {step}"
        );
    }
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_second_connect_of_an_outlet_gets_wrong_state_and_changes_nothing() {
    let mut hv = booted();
    let outlet = boot_loop_ids()[0];
    let first = call(CONNECT_IRQ_PLUG_EXT, 0, &[1, 0, 0, outlet, 0]);
    assert!(matches!(
        hv.dispatch_with(&first, &storage()),
        HypercallDisposition::Answered(HypercallAnswer { r3: 0, .. })
    ));
    let once = (hv.event_receive_ports().clone(), hv.sync_term());
    let again = call(CONNECT_IRQ_PLUG_EXT, 0, &[1, 0, 1, outlet, 0]);
    assert!(matches!(
        hv.dispatch_with(&again, &storage()),
        HypercallDisposition::Answered(HypercallAnswer {
            r3: IRQ_PLUG_PORT_CONNECTED_R3,
            ..
        })
    ));
    assert_eq!((hv.event_receive_ports().clone(), hv.sync_term()), once);
}
