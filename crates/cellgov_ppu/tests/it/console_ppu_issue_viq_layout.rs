//! CellGov's PPU timing against the `ppu_issue_viq_layout` console
//! capture: sixteen independent `addi` timed beside a sibling whose round
//! starts with nothing, one `fdiv`, `nop` padding or `addi` padding, and
//! then runs 256 independent `addi`.
//!
//! The probe repeats the `ppu_issue_viq_depth` sibling of no divides, and
//! moves its inner loop across the line with padding in place of the
//! divides.

use super::console_ppu_issue_viq_depth::{replay, BAND};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_viq_layout/ps3/cech20-cex-493/observation.json";

/// The payload's word 510 when the probe wrote every word.
const DONE: u64 = 0x5050_564C;

/// The runs that count in each phase.
const REPS: usize = 15;

/// The phases with a sibling.
const SIBLINGS: usize = 10;

/// The phase whose round starts with nothing.
const NOTHING: usize = 1;

/// The phase whose round starts with one `fdiv`.
const ONE_DIVIDE: usize = 2;

/// The phase whose 30 `nop` move the inner loop to the next line.
const NEXT_LINE: usize = 10;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4088);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[510], DONE, "the probe wrote every word");
    assert_eq!(w[508], REPS as u64, "fifteen runs that count a phase");
    for p in 0..=SIBLINGS {
        let base = 390 + 9 * p;
        assert_eq!(w[base], REPS as u64, "phase {p} timed fifteen runs");
        if p > 0 {
            assert_eq!(w[base + 4], 0, "phase {p}'s sibling was created");
            assert_eq!(w[base + 5], 0, "phase {p}'s sibling joined");
            assert_eq!(w[base + 6], SIBLING_EXIT, "phase {p}'s sibling exited");
            assert_eq!(w[base + 7], 1, "phase {p}'s sibling ran");
            assert_eq!(w[base + 8], 0, "phase {p}'s sleep returned");
        }
    }
    w
}

/// Phase `p`'s median run, in cycles an iteration of the timed loop.
fn median(w: &[u64], p: usize) -> f64 {
    let mut runs: Vec<f64> = (0..REPS)
        .map(|r| (w[2 * (REPS * p + r)] * PPU_CYCLES_PER_TB_TICK) as f64 / w[509] as f64)
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[REPS / 2]
}

#[test]
fn on_the_console_a_sibling_of_no_divides_costs_the_loop_what_one_divide_costs() {
    let w = console_words();
    let one = median(&w, ONE_DIVIDE);
    for p in (NOTHING..NEXT_LINE).filter(|&p| p != ONE_DIVIDE) {
        let padded = median(&w, p);
        assert!(
            (padded - one).abs() / one < 0.02,
            "phase {p}: {padded:.2} cycles an iteration, one divide {one:.2}"
        );
    }
    // With its inner loop at the next line, the sibling costs the loop
    // about 3% more than one divide.
    let next = median(&w, NEXT_LINE);
    assert!(
        (next - one).abs() / one <= BAND,
        "the next line: {next:.2} cycles an iteration, one divide {one:.2}"
    );
}

#[test]
fn beside_a_sibling_of_no_divides_the_model_times_the_loop_near_the_console() {
    let w = console_words();
    let console = median(&w, NOTHING);
    let cellgov = replay(0, 0, w[509]);
    assert!(
        (cellgov - console).abs() / console <= BAND,
        "no divides: CellGov {cellgov:.2} cycles an iteration, the console {console:.2}"
    );
}
