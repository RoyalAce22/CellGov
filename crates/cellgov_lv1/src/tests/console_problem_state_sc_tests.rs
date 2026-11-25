//! The hypervisor answers the `lv1_problem_state_sc` probe's five
//! problem-state `sc 1` calls as the reference console does: the same
//! r3 to r10 for each, read from its committed capture.

use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

const PROBE: &str = "lv1_problem_state_sc";

/// The numbers the probe sends in r11, in the order `ppu/main.c` runs
/// them.
const NUMBERS: [u64; 5] = [256, 0xFFFF, 0x8000_0000_0000_0000, 14, 74];

/// The bytes of one case in the payload: r11, then r3 to r10.
const CASE_BYTES: usize = 72;

/// The value the probe puts in register `reg` before case `case`.
fn mark(reg: u64, case: u64) -> u64 {
    0xC0DE_0000_0000_0000 | (reg << 8) | case
}

/// The payload of the frame the reference console wrote for [`PROBE`]:
/// the bytes after the `CGOV` magic and the big-endian length.
///
/// The `ps3_captures` guard in `cellgov_compare` holds each committed
/// frame to its observation and provenance.
fn console_payload() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    frame[8..].to_vec()
}

fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

/// The console's r11 and its r3 to r10 after the call, for each case.
fn console_cases() -> Vec<(u64, [u64; 8])> {
    let payload = console_payload();
    assert_eq!(payload.len(), 8 + NUMBERS.len() * CASE_BYTES);
    assert_eq!(word(&payload, 0), NUMBERS.len() as u64, "case count");
    (0..NUMBERS.len())
        .map(|case| {
            let at = 8 + case * CASE_BYTES;
            let regs = std::array::from_fn(|r| word(&payload, at + 8 + r * 8));
            (word(&payload, at), regs)
        })
        .collect()
}

/// CellGov's r3 to r10 after a 4.93 hypervisor answers case `case`, as
/// the runtime applies an answer:
/// - r3 takes the answer's r3;
/// - r4 to r10 take the answer's values, or keep the marks where the
///   answer gives none.
fn cellgov_registers(case: usize) -> [u64; 8] {
    let inputs = std::array::from_fn(|r| mark(3 + r as u64, case as u64));
    let request = HypercallRequest {
        unit: UnitId::new(1),
        number: NUMBERS[case],
        inputs,
        pc: 0x1_036c,
        lr: 0,
        thread: Some(0),
        problem_state: true,
        tick: GuestTicks::new(0),
    };
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let answer = match hv.dispatch(&request) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("case {case}: {refusal}"),
    };
    let mut regs = inputs;
    regs[0] = answer.r3;
    if let Some(outputs) = answer.outputs() {
        regs[1..].copy_from_slice(&outputs);
    }
    regs
}

fn assert_cases_match(cases: std::ops::Range<usize>) {
    let console = console_cases();
    for case in cases {
        let (number, regs) = console[case];
        assert_eq!(number, NUMBERS[case], "case {case}: r11");
        assert_eq!(
            cellgov_registers(case),
            regs,
            "case {case}: r3 to r10 for number {number:#x}"
        );
    }
}

#[test]
fn a_problem_state_number_past_the_table_answers_as_the_console_does() {
    assert_cases_match(0..3);
}

#[test]
fn a_problem_state_call_to_an_entry_in_the_table_answers_as_the_console_does() {
    assert_cases_match(3..5);
}
