//! The port id of the call in the 4.93 kernel's create function of its
//! boot logical SPE, after the eight per-thread calls of the boot.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::spe::LOGICAL_SPE_OBJECT_CAPTURE;
use cellgov_time::GuestTicks;

/// The `sc` of the call in the boot SPE's create function
/// [OFW-4.93 lv2_kernel a:0x800000000004f218].
const SPE: u64 = 0x8000_0000_0004_f218;

/// The port id at +88 of the boot SPE object in the committed capture
/// `tests/micro/lv2_logical_spe_object`. Both reads of the doubleword
/// must agree.
fn spe_port_id() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LOGICAL_SPE_OBJECT_CAPTURE)
        .join("peek/cech20-cex-493/spe_object.tsv");
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
        .find(|row| row.get(1) == Some(&"port"))
        .unwrap_or_else(|| panic!("{name}: no row for port"));
    assert_eq!(row.len(), 6, "{name}: port columns");
    assert_eq!((row[3], row[4]), ("2", "yes"), "{name}: port reads");
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: port: {e}"))
}

/// The eight per-thread calls in the order of their port ids, which the
/// capture `lv2_event_port_ids` reads: the record call
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c] and calls A, B and C
/// [OFW-4.93 lv2_kernel a:0x80000000002656d0]
/// [OFW-4.93 lv2_kernel a:0x8000000000265784]
/// [OFW-4.93 lv2_kernel a:0x8000000000265840].
const PER_THREAD: [(u8, u64); 8] = [
    (0, 0x8000_0000_0000_7d8c),
    (1, 0x8000_0000_0000_7d8c),
    (1, 0x8000_0000_0026_56d0),
    (1, 0x8000_0000_0026_5784),
    (1, 0x8000_0000_0026_5840),
    (0, 0x8000_0000_0026_56d0),
    (0, 0x8000_0000_0026_5784),
    (0, 0x8000_0000_0026_5840),
];

fn construct(thread: u8, pc: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0; 8],
        pc,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// A 4.93 hypervisor after the eight per-thread calls.
fn after_per_thread_calls() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for (thread, pc) in PER_THREAD {
        assert!(
            matches!(
                hv.dispatch(&construct(thread, pc)),
                HypercallDisposition::Answered(_)
            ),
            "thread {thread} at {pc:#018x}"
        );
    }
    hv
}

#[test]
fn thread_0_s_boot_spe_call_gets_the_captured_id_and_the_hypervisor_keeps_it() {
    let mut hv = after_per_thread_calls();
    let id = spe_port_id();
    let mut expected = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    expected.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    assert_eq!(
        hv.dispatch(&construct(0, SPE)),
        HypercallDisposition::Answered(expected)
    );
    let held: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(held, [3, 4, 5, 6, 7, 8, 9, 10, id]);
    assert_eq!(hv.refusal(), None);
}
