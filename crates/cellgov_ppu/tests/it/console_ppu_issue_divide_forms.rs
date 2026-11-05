//! CellGov's PPU timing of the overflow-enable and recording forms of
//! signed divides, and of `divw` on operands whose high words are not
//! sign extensions, against the `ppu_issue_divide_forms` console capture.
//!
//! The probe times twelve loops of eight `divw`, `divwo`, `divw.`,
//! `divdo` or `divd.` of r15 by r16 into r17: an `addi` control, the most
//! negative value by -1 and by 1 in each form, and `divw` of a low word
//! 0x8000_0000 by a low word -1 with zero and with arbitrary high words.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_divide_forms/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 27 when the probe wrote every word.
const DONE: u64 = 0x5050_4446;

/// The loops the probe times.
const LOOPS: usize = 12;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 28);
    assert_eq!(words[27], DONE, "the probe wrote every word");
    words
}

/// Loop `l`'s cycles an iteration on the console: the fastest run and the
/// slowest.
fn console_cycles(words: &[u64], l: usize) -> (f64, f64) {
    let passes = words[26];
    let per = |tb: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / passes as f64;
    (per(words[2 * l]), per(words[2 * l + 1]))
}

/// `addi r17,r17,1`.
const ADDI: u32 = 0x3a31_0001;
/// `divw r17,r15,r16`.
const DIVW: u32 = 0x7e2f_83d6;
/// `divwo r17,r15,r16`.
const DIVWO: u32 = 0x7e2f_87d6;
/// `divw. r17,r15,r16`.
const DIVW_RC: u32 = 0x7e2f_83d7;
/// `divdo r17,r15,r16`.
const DIVDO: u32 = 0x7e2f_87d2;
/// `divd. r17,r15,r16`.
const DIVD_RC: u32 = 0x7e2f_83d3;

const CHAIN_ADDI: [u32; 9] = eight(ADDI);
const CHAIN_DIVW: [u32; 9] = eight(DIVW);
const CHAIN_DIVWO: [u32; 9] = eight(DIVWO);
const CHAIN_DIVW_RC: [u32; 9] = eight(DIVW_RC);
const CHAIN_DIVDO: [u32; 9] = eight(DIVDO);
const CHAIN_DIVD_RC: [u32; 9] = eight(DIVD_RC);

const LOW_MIN: u64 = 0x8000_0000;
const LOW_MINUS_ONE: u64 = 0xffff_ffff;
const WORD_MIN: u64 = 0xffff_ffff_8000_0000;
const DWORD_MIN: u64 = 0x8000_0000_0000_0000;
const MINUS_ONE: u64 = u64::MAX;
const HIGH_MIN: u64 = 0x1234_5678_8000_0000;
const HIGH_MINUS_ONE: u64 = 0x9abc_def0_ffff_ffff;

const fn chain(loop_index: usize, words: &'static [u32], seeds: &'static [Seed]) -> Replay {
    Replay {
        loop_index,
        words,
        seeds,
    }
}

/// The probe's loops, in its order, and their names.
const PROBE_LOOPS: [(&str, Replay); LOOPS] = [
    ("addi", chain(0, &CHAIN_ADDI, &[])),
    (
        "divw low MIN / low -1",
        chain(
            1,
            &CHAIN_DIVW,
            &[Seed::Gpr(15, LOW_MIN), Seed::Gpr(16, LOW_MINUS_ONE)],
        ),
    ),
    (
        "divw low MIN / 1",
        chain(2, &CHAIN_DIVW, &[Seed::Gpr(15, LOW_MIN), Seed::Gpr(16, 1)]),
    ),
    (
        "divwo MIN / -1",
        chain(
            3,
            &CHAIN_DIVWO,
            &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divwo MIN / 1",
        chain(
            4,
            &CHAIN_DIVWO,
            &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, 1)],
        ),
    ),
    (
        "divw. MIN / -1",
        chain(
            5,
            &CHAIN_DIVW_RC,
            &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divw. MIN / 1",
        chain(
            6,
            &CHAIN_DIVW_RC,
            &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, 1)],
        ),
    ),
    (
        "divdo MIN / -1",
        chain(
            7,
            &CHAIN_DIVDO,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divdo MIN / 1",
        chain(
            8,
            &CHAIN_DIVDO,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, 1)],
        ),
    ),
    (
        "divd. MIN / -1",
        chain(
            9,
            &CHAIN_DIVD_RC,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divd. MIN / 1",
        chain(
            10,
            &CHAIN_DIVD_RC,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, 1)],
        ),
    ),
    (
        "divw high-word MIN / high-word -1",
        chain(
            11,
            &CHAIN_DIVW,
            &[Seed::Gpr(15, HIGH_MIN), Seed::Gpr(16, HIGH_MINUS_ONE)],
        ),
    ),
];

/// The largest share of a loop's fastest console run the model may differ
/// by, when the model falls outside the console's spread.
const TOLERANCE: f64 = 0.20;

#[test]
fn every_divide_form_loop_costs_what_it_costs_on_the_console() {
    let w = console_words();
    for (l, (name, r)) in PROBE_LOOPS.iter().enumerate() {
        let cellgov = replay(r, 0).expect("the loop replays");
        let (fastest, slowest) = console_cycles(&w, l);
        assert!(
            (fastest..=slowest).contains(&cellgov)
                || (cellgov - fastest).abs() / fastest <= TOLERANCE,
            "{name}: CellGov {cellgov:.2} cycles an iteration, the console {fastest:.2} to \
             {slowest:.2}",
        );
    }
}

#[test]
fn every_form_of_the_most_negative_value_by_minus_one_saves_what_it_saves_on_the_console() {
    let w = console_words();
    for (by_minus_one, by_one) in [(1, 2), (3, 4), (5, 6), (7, 8), (9, 10), (11, 2)] {
        let console = |l: usize| console_cycles(&w, l).0;
        let cellgov = |l: usize| replay(&PROBE_LOOPS[l].1, 0).expect("the loop replays");
        let name = PROBE_LOOPS[by_minus_one].0;
        let console_saves = console(by_one) - console(by_minus_one);
        let cellgov_saves = cellgov(by_one) - cellgov(by_minus_one);
        assert!(
            (cellgov_saves - console_saves).abs() / console_saves <= TOLERANCE,
            "{name}: CellGov saves {cellgov_saves:.1} cycles an iteration over by 1, the \
             console {console_saves:.1}",
        );
    }
}
