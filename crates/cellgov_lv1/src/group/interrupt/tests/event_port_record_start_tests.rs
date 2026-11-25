//! The port id of the call that each thread's start makes with the
//! thread's record, after the boot SPE's call on thread 0.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::event_port::{game_os_port_id, RECORD_PORT_CAPTURE};
use cellgov_time::GuestTicks;

/// The `sc` of the call [OFW-4.93 lv2_kernel a:0x8000000000027d20].
const RECORD_START: u64 = 0x8000_0000_0002_7d20;

/// The `sc` of the call in the boot SPE's create function
/// [OFW-4.93 lv2_kernel a:0x800000000004f218].
const SPE: u64 = 0x8000_0000_0004_f218;

/// The eight per-thread calls of the boot in the order of their port ids:
/// the record call [OFW-4.93 lv2_kernel a:0x8000000000007d8c] and calls
/// A, B and C [OFW-4.93 lv2_kernel a:0x80000000002656d0]
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

/// The doubleword `field` in the committed capture. Both reads of the
/// doubleword must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(RECORD_PORT_CAPTURE)
        .join("peek/cech20-cex-493/record_ports.tsv");
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

/// The port id at +0x38 of the record of hardware thread `thread` in the
/// committed capture. The record's +0x10 must name that thread.
fn record_port_id(thread: u8) -> u64 {
    assert_eq!(
        captured(&format!("ctrl_{thread}")),
        u64::from(thread),
        "the hardware thread of record {thread}"
    );
    captured(&format!("port_{thread}"))
}

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

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

/// A 4.93 hypervisor after the eight per-thread calls and the boot SPE's
/// call.
fn after_boot_spe_call() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for (thread, pc) in PER_THREAD.into_iter().chain([(0, SPE)]) {
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
fn thread_0_s_record_call_after_the_boot_spe_call_gets_the_id_at_0x38_of_its_record() {
    let mut hv = after_boot_spe_call();
    let id = record_port_id(0);
    assert_eq!(hv.dispatch(&construct(0, RECORD_START)), constructed(id));
    let expected: Vec<u64> = PER_THREAD
        .into_iter()
        .chain([(0, SPE)])
        .map(|(thread, pc)| {
            game_os_port_id("4.93", thread, pc)
                .unwrap_or_else(|| panic!("the port id of thread {thread} at {pc:#018x}"))
        })
        .chain([id])
        .collect();
    let held: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(held, expected);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn thread_1_s_record_call_gets_the_id_at_0x38_of_its_record() {
    let mut hv = after_boot_spe_call();
    assert_eq!(
        hv.dispatch(&construct(1, RECORD_START)),
        constructed(record_port_id(1))
    );
    assert_eq!(hv.refusal(), None);
}
