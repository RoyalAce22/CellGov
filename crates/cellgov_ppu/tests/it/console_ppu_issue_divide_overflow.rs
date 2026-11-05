//! CellGov's PPU timing of signed divides on their overflow cases against
//! the `ppu_issue_divide_overflow` console capture.
//!
//! The probe times eight loops of eight `divw` or `divd` of r15 by r16
//! into r17: an `addi` control, the most negative value by -1, by 1 and
//! by 0 in each width, and the most positive word by -1. The replays run
//! each loop through the execution unit and its issue model.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_divide_overflow/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 19 when the probe wrote every word.
const DONE: u64 = 0x5050_444f;

/// The loops the probe times.
const LOOPS: usize = 8;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 24);
    assert_eq!(words[19], DONE, "the probe wrote every word");
    words
}

/// Loop `l`'s cycles an iteration on the console: the fastest run and the
/// slowest.
fn console_cycles(words: &[u64], l: usize) -> (f64, f64) {
    let passes = words[18];
    let per = |tb: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / passes as f64;
    (per(words[2 * l]), per(words[2 * l + 1]))
}

/// `addi r17,r17,1`.
const ADDI: u32 = 0x3a31_0001;
/// `divw r17,r15,r16`.
const DIVW: u32 = 0x7e2f_83d6;
/// `divd r17,r15,r16`.
const DIVD: u32 = 0x7e2f_83d2;

const CHAIN_ADDI: [u32; 9] = eight(ADDI);
const CHAIN_DIVW: [u32; 9] = eight(DIVW);
const CHAIN_DIVD: [u32; 9] = eight(DIVD);

const WORD_MIN: u64 = 0xffff_ffff_8000_0000;
const WORD_MAX: u64 = 0x7fff_ffff;
const DWORD_MIN: u64 = 0x8000_0000_0000_0000;
const MINUS_ONE: u64 = u64::MAX;

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
        "divw MIN / -1",
        chain(
            1,
            &CHAIN_DIVW,
            &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divw MIN / 1",
        chain(2, &CHAIN_DIVW, &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, 1)]),
    ),
    (
        "divw MIN / 0",
        chain(3, &CHAIN_DIVW, &[Seed::Gpr(15, WORD_MIN), Seed::Gpr(16, 0)]),
    ),
    (
        "divd MIN / -1",
        chain(
            4,
            &CHAIN_DIVD,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
    (
        "divd MIN / 1",
        chain(
            5,
            &CHAIN_DIVD,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, 1)],
        ),
    ),
    (
        "divd MIN / 0",
        chain(
            6,
            &CHAIN_DIVD,
            &[Seed::Gpr(15, DWORD_MIN), Seed::Gpr(16, 0)],
        ),
    ),
    (
        "divw MAX / -1",
        chain(
            7,
            &CHAIN_DIVW,
            &[Seed::Gpr(15, WORD_MAX), Seed::Gpr(16, MINUS_ONE)],
        ),
    ),
];

/// The largest share of a loop's fastest console run the model may differ
/// by, when the model falls outside the console's spread.
const TOLERANCE: f64 = 0.20;

#[test]
fn every_divide_loop_costs_what_it_costs_on_the_console() {
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
fn the_most_negative_value_by_minus_one_costs_what_a_zero_divisor_costs() {
    let w = console_words();
    for (by_minus_one, by_one, by_zero) in [(1, 2, 3), (4, 5, 6)] {
        let console = |l: usize| console_cycles(&w, l).0;
        let cellgov = |l: usize| replay(&PROBE_LOOPS[l].1, 0).expect("the loop replays");
        let name = PROBE_LOOPS[by_minus_one].0;
        assert!(
            (console(by_minus_one) - console(by_zero)).abs() / console(by_zero) < 0.05,
            "{name}: the console {:.1}, by zero {:.1}",
            console(by_minus_one),
            console(by_zero),
        );
        assert!(console(by_minus_one) * 2.0 < console(by_one));
        assert_eq!(
            cellgov(by_minus_one),
            cellgov(by_zero),
            "{name}: CellGov times it as a divide by zero",
        );
        assert!(
            cellgov(by_minus_one) * 2.0 < cellgov(by_one),
            "{name}: CellGov {:.1}, by 1 {:.1}",
            cellgov(by_minus_one),
            cellgov(by_one),
        );
    }
}
