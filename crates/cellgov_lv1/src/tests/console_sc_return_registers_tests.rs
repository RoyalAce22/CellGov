//! The hypervisor's two returns leave r0, r11, r12 and CR as the
//! reference console's lv1 does after the `lv1_sc_return_registers`
//! probe's ten problem-state `sc 1` calls, read from its committed
//! capture.

use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

use super::{Compare, ReturnPair, ReturnRegisters};
use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

const PROBE: &str = "lv1_sc_return_registers";

/// The numbers the probe sends, in the order `ppu/main.c` runs them:
/// each once with XER at 0, then once with XER\[SO\] and XER\[CA\] set.
const NUMBERS: [u64; 5] = [256, 0xFFFF, 0x8000_0000_0000_0000, 14, 74];

/// The CR the probe sets before each call.
const CR_IN: u32 = 0x6E7D_ACBF;

/// XER\[SO\] as `mtxer` sets it.
const XER_SO: u64 = 0x8000_0000;

/// The bytes of one case in the payload: the number, the XER sent, the
/// address 20 bytes before the `sc`, then r0, r3, r11, r12, CR, XER, LR
/// and CTR.
const CASE_BYTES: usize = 88;

/// One case as the console ran it.
#[derive(Debug)]
struct ConsoleCase {
    number: u64,
    xer: u64,
    sc: u64,
    r0: u64,
    r3: u64,
    r11: u64,
    r12: u64,
    cr: u32,
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

fn console_cases() -> Vec<ConsoleCase> {
    let payload = console_payload();
    let count = 2 * NUMBERS.len();
    assert_eq!(payload.len(), 8 + count * CASE_BYTES);
    assert_eq!(word(&payload, 0), count as u64, "case count");
    (0..count)
        .map(|case| {
            let at = 8 + case * CASE_BYTES;
            let w = |i: usize| word(&payload, at + 8 * i);
            ConsoleCase {
                number: w(0),
                xer: w(1),
                sc: w(2) + 20,
                r0: w(3),
                r3: w(4),
                r11: w(5),
                r12: w(6),
                cr: w(7) as u32,
            }
        })
        .collect()
}

/// The CR after a return that leaves `cr` in the fields, from a caller
/// whose CR was [`CR_IN`] and whose XER was `xer`.
fn cr_after(cr: &[Option<Compare>; 8], xer: u64) -> u32 {
    let so = u32::from(xer & XER_SO != 0);
    (0..8).fold(CR_IN, |value, field| {
        let Some(result) = cr[field] else {
            return value;
        };
        let bits = match result {
            Compare::Less => 0b1000,
            Compare::Greater => 0b0100,
            Compare::Equal => 0b0010,
        } | so;
        let shift = 28 - 4 * field;
        (value & !(0xF << shift)) | (bits << shift)
    })
}

/// The r3 and the return registers of a 4.93 hypervisor's answer to a
/// problem-state call of `number` from the `sc` at `sc`.
fn cellgov_answer(number: u64, sc: u64) -> (u64, ReturnRegisters) {
    let request = HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs: [0; 8],
        pc: sc,
        lr: 0,
        thread: Some(0),
        problem_state: true,
        tick: GuestTicks::new(0),
    };
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    match hv.dispatch(&request) {
        HypercallDisposition::Answered(answer) => {
            (answer.r3, answer.return_registers(&request, Some("4.93")))
        }
        HypercallDisposition::Refused(refusal) => panic!("{number:#x}: {refusal}"),
    }
}

fn assert_cases_match(numbers: std::ops::Range<usize>) {
    let console = console_cases();
    let mut checked = 0;
    for half in [0, NUMBERS.len()] {
        for case in numbers.clone().map(|i| half + i) {
            let c = &console[case];
            assert_eq!(
                c.number,
                NUMBERS[case % NUMBERS.len()],
                "case {case}: number"
            );
            let (r3, returned) = cellgov_answer(c.number, c.sc);
            assert_eq!(r3, c.r3, "case {case}: r3");
            assert_eq!(returned.r0, c.r0, "case {case}: r0");
            match returned.r11_r12 {
                ReturnPair::Values { r11, r12 } => {
                    assert_eq!((r11, r12), (c.r11, c.r12), "case {case}: r11 and r12");
                }
                ReturnPair::SaveRestore => {
                    assert_eq!(c.r11, c.sc + 4, "case {case}: r11 is SRR0");
                }
            }
            assert_eq!(
                cr_after(&returned.cr, c.xer),
                c.cr,
                "case {case}: CR for number {:#x}, XER {:#x}",
                c.number,
                c.xer
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 2 * numbers.len());
}

#[test]
fn the_past_the_table_return_leaves_r0_r11_r12_and_cr_as_the_console_does() {
    assert_cases_match(0..3);
}

#[test]
fn the_return_through_hrfid_leaves_r0_r11_and_cr_as_the_console_does() {
    assert_cases_match(3..5);
}
