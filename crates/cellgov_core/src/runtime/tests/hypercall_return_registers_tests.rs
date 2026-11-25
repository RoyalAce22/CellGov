//! The runtime gives the caller r4 to r10 from every answer that lv1's
//! return path loads, and leaves them to the caller where it does not.

use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;
use cellgov_ps3_abi::lv1::dispatch::UNKNOWN_HYPERCALL_R3;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_trace::TraceRecord;

use super::{records, runtime, step};

/// The value the caller holds in each of r3 to r10 before its call.
const MARK: u64 = 0xC0DE_0000_0000_C0DE;

/// The `HypercallReturned` records of `records`, as `(status, outputs,
/// written)`.
fn returned(records: Vec<TraceRecord>) -> Vec<(u64, [u64; 7], u8)> {
    records
        .into_iter()
        .filter_map(|r| match r {
            TraceRecord::HypercallReturned {
                status,
                outputs,
                written,
                ..
            } => Some((status, outputs, written)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_one_output_answer_writes_zero_to_r5_to_r10_over_the_callers_values() {
    let (mut rt, received) = runtime(GET_LOGICAL_PARTITION_ID, [MARK; 8]);
    step(&mut rt).expect("the hypercall step");
    step(&mut rt).expect("the caller's next step");
    let mut writes = vec![(4, LPAR_ID_GAME_OS)];
    writes.extend((5..=10).map(|reg| (reg, 0)));
    assert_eq!(*received.borrow(), Some((Some(0), writes)));
    let outputs = [LPAR_ID_GAME_OS, 0, 0, 0, 0, 0, 0];
    assert_eq!(returned(records(&rt)), vec![(0, outputs, 7)]);
}

#[test]
fn a_number_past_the_table_traces_no_written_output() {
    let (mut rt, received) = runtime(256, [MARK; 8]);
    rt.set_hypervisor_firmware("4.93");
    step(&mut rt).expect("the hypercall step");
    step(&mut rt).expect("the caller's next step");
    assert_eq!(
        *received.borrow(),
        Some((Some(UNKNOWN_HYPERCALL_R3), Vec::new()))
    );
    assert_eq!(
        returned(records(&rt)),
        vec![(UNKNOWN_HYPERCALL_R3, [0; 7], 0)]
    );
    assert_eq!(rt.hypervisor().refusal(), None);
}
