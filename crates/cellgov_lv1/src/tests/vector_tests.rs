//! lv1's vector answers and the null backend, by firmware and processor
//! state.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_ps3_abi::lv1::dispatch::{PROBLEM_STATE_DENIED_R3, UNKNOWN_HYPERCALL_R3};
use cellgov_ps3_abi::lv1::table::{
    table_entry, TableEntry, HYPERCALL_TABLE_SITES, INVALID_HYPERCALL_R3,
};
use cellgov_time::GuestTicks;

use super::{null_backend, vector_disposition};
use crate::{
    answer, AddressSpaceState, EventReceivePortState, HypercallAnswer, HypercallDisposition,
    HypercallRefusal, HypercallRequest, HypercallWitness, Hypervisor, IrqStateBitmapState,
    MemoryState, ANSWERED,
};

fn request(number: u64, problem_state: bool) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(2),
        number,
        inputs: [7; 8],
        pc: 0x8000_0000_0001_0000,
        lr: 0,
        thread: Some(0),
        problem_state,
        tick: GuestTicks::new(40),
    }
}

fn answered(r3: u64, outputs: Vec<u64>) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer {
        r3,
        outputs,
        effects: Vec::new(),
        console: None,
        region: None,
        address_space: None,
        page_table: None,
        interrupt: None,
    })
}

fn unmodelled(r: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::Unmodelled {
        number: r.number,
        unit: r.unit,
        pc: r.pc,
    })
}

#[test]
fn a_number_past_the_table_answers_not_implemented_at_each_site_firmware() {
    assert_eq!(HYPERCALL_TABLE_SITES.len(), 5);
    for site in HYPERCALL_TABLE_SITES {
        for number in [256, 0x1_0000, u64::MAX] {
            for problem_state in [false, true] {
                assert_eq!(
                    vector_disposition(&request(number, problem_state), Some(site.firmware)),
                    Some(answered(UNKNOWN_HYPERCALL_R3, Vec::new())),
                    "{} number {number} problem state {problem_state}",
                    site.firmware
                );
            }
        }
    }
}

#[test]
fn a_number_past_the_table_reaches_the_null_backend_where_no_image_gives_the_count() {
    for firmware in [None, Some("1.50"), Some("4.92")] {
        let r = request(256, false);
        assert_eq!(vector_disposition(&r, firmware), None, "{firmware:?}");
        assert_eq!(
            answer(
                &r,
                firmware,
                &MemoryState::default(),
                &AddressSpaceState::default(),
                &IrqStateBitmapState::default(),
                &EventReceivePortState::default(),
                &GuestMemory::new(0)
            ),
            unmodelled(&r),
            "{firmware:?}"
        );
    }
}

#[test]
fn every_problem_state_call_below_256_answers_no_privilege_at_4_93() {
    let mut checked = 0;
    for number in 0..256 {
        let r = request(number, true);
        assert_eq!(
            answer(
                &r,
                Some("4.93"),
                &MemoryState::default(),
                &AddressSpaceState::default(),
                &IrqStateBitmapState::default(),
                &EventReceivePortState::default(),
                &GuestMemory::new(0)
            ),
            answered(PROBLEM_STATE_DENIED_R3, vec![0; 7]),
            "number {number}"
        );
        checked += 1;
    }
    assert_eq!(checked, 256);
}

#[test]
fn a_problem_state_call_refuses_where_no_capture_gives_the_table() {
    for firmware in [None, Some("3.70")] {
        let r = request(74, true);
        assert_eq!(
            vector_disposition(&r, firmware),
            Some(HypercallDisposition::Refused(
                HypercallRefusal::ProblemState {
                    number: 74,
                    unit: r.unit,
                    pc: r.pc,
                }
            )),
            "{firmware:?}"
        );
    }
}

#[test]
fn a_supervisor_call_below_256_reaches_its_table_entry() {
    assert_eq!(vector_disposition(&request(14, false), Some("4.93")), None);
    assert_eq!(vector_disposition(&request(255, false), Some("4.93")), None);
}

#[test]
fn an_invalid_entry_answers_the_invalid_function_with_r4_to_r10_cleared_at_4_93() {
    let r = request(14, false);
    assert_eq!(
        null_backend(&r, Some("4.93")),
        answered(INVALID_HYPERCALL_R3, vec![0; 7])
    );
}

#[test]
fn a_number_lv1_implements_and_cellgov_does_not_model_stops_the_run() {
    let r = request(3, false);
    assert_eq!(table_entry("4.93", 3), Some(TableEntry::Handler(0x2ECDE8)));
    assert_eq!(null_backend(&r, Some("4.93")), unmodelled(&r));
}

#[test]
fn an_entry_invalid_at_4_93_stops_the_run_at_another_firmware() {
    let r = request(14, false);
    for firmware in [None, Some("3.70")] {
        assert_eq!(null_backend(&r, firmware), unmodelled(&r), "{firmware:?}");
    }
}

#[test]
fn every_answered_number_has_a_handler_in_the_live_4_93_table() {
    assert_eq!(ANSWERED.len(), 14);
    for &number in ANSWERED {
        assert!(
            matches!(table_entry("4.93", number), Some(TableEntry::Handler(_))),
            "{number}"
        );
    }
}

#[test]
fn the_invalid_answer_counts_a_witness_and_keeps_no_refusal() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    hv.dispatch(&request(14, false));
    hv.dispatch(&request(256, false));
    assert_eq!(hv.refusal(), None);
    let witnesses: Vec<_> = hv.witnesses().iter().map(|(n, w)| (*n, *w)).collect();
    let witness = HypercallWitness {
        hits: 1,
        first_tick: GuestTicks::new(40),
    };
    assert_eq!(witnesses, vec![(14, witness), (256, witness)]);
}

#[test]
fn a_problem_state_call_to_an_answered_number_counts_no_witness() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let d = hv.dispatch(&request(74, true));
    assert_eq!(d, answered(PROBLEM_STATE_DENIED_R3, vec![0; 7]));
    assert!(hv.witnesses().is_empty());
}

#[test]
fn a_problem_state_repository_read_that_the_vector_disposes_of_records_no_read() {
    for firmware in ["4.93", "3.70"] {
        let mut hv = Hypervisor::new();
        hv.set_firmware(firmware);
        hv.dispatch(&request(READ_REPOSITORY_NODE, true));
        assert!(hv.repository_reads().is_empty(), "{firmware}");
    }
}
