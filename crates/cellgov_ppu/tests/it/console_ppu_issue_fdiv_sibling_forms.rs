//! CellGov's PPU timing against the `ppu_issue_fdiv_sibling_forms`
//! console capture: sixteen independent `addi` beside a sibling of
//! dependent `fdiv` rounds, in the forms the `ppu_issue_mix_switch` and
//! `ppu_issue_shared` probes differ by.

use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_fdiv_sibling_forms/ps3/cech20-cex-493/observation.json";

/// The phases the probe times.
const PHASES: usize = 5;

/// The runs that count in each phase.
const REPS: usize = 15;

/// The timed loop's iterations in each half.
const HALF: u64 = 2048;

/// The payload's word 363 when the probe wrote every word.
const DONE: u64 = 0x5050_4654;

/// The payload of the capture at `path` as big-endian words.
fn words_at(path: &str) -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let words: Vec<u64> = observation.memory_regions[0]
        .data
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 364);
    assert_eq!(words[363], DONE, "the probe wrote every word");
    assert_eq!((words[361], words[362]), (REPS as u64, HALF));
    words
}

/// Phase `p`'s counted runs, in core cycles an iteration of the timed
/// loop.
fn runs(words: &[u64], p: usize) -> Vec<f64> {
    let counted = words[300 + 2 * p] as usize;
    (0..counted)
        .map(|r| {
            let at = 4 * (REPS * p + r);
            let ticks = words[at] + words[at + 1];
            (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / (2 * HALF) as f64
        })
        .collect()
}

/// The phases with a sibling.
const SIBLING_PHASES: std::ops::Range<usize> = 1..PHASES;

/// The sibling's exit value.
const SIBLING_EXIT: u64 = 0x5157;

/// The phases that time the loop as one loop of `2 * HALF` iterations.
const ONE_LOOP_PHASES: std::ops::Range<usize> = 3..PHASES;

/// Phase `p`'s timed parts over its counted runs, in core cycles an
/// iteration: each run's two halves, or its one loop.
fn parts(words: &[u64], p: usize) -> Vec<f64> {
    let counted = words[300 + 2 * p] as usize;
    let (per_run, iterations) = if ONE_LOOP_PHASES.contains(&p) {
        (1, 2 * HALF)
    } else {
        (2, HALF)
    };
    (0..counted)
        .flat_map(|r| (0..per_run).map(move |j| words[4 * (REPS * p + r) + j]))
        .map(|ticks| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64)
        .collect()
}

/// The fastest and slowest of `xs`.
fn range(xs: &[f64]) -> (f64, f64) {
    xs.iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)))
}

#[test]
fn every_sibling_ran_and_every_phase_kept_its_runs() {
    let w = words_at(CONSOLE);
    assert_eq!(runs(&w, 0).len(), REPS);
    for p in SIBLING_PHASES {
        let b = 310 + 10 * p;
        assert_eq!(
            (w[b + 4], w[b + 5], w[b + 6], w[b + 7], w[b + 8]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {p}: the sibling's create rc, join rc, exit value, start and sleep rc",
        );
        // With no rounds over the sleep the probe counts every run.
        assert!(w[b] > 0, "phase {p}: the sibling's rounds over the sleep");
        assert_eq!(runs(&w, p).len(), REPS, "phase {p}");
    }
}

#[test]
fn beside_a_steady_fdiv_sibling_every_form_times_the_loop_from_under_250_to_over_700_cycles() {
    // The `ppu_issue_shared` capture times this loop at about 195 and
    // the `ppu_issue_mix_switch` capture at about 720: each probe caught
    // parts of one end of the spread more often.
    let w = words_at(CONSOLE);
    for p in SIBLING_PHASES {
        let (lo, hi) = range(&parts(&w, p));
        assert!(
            lo < 250.0 && hi > 700.0,
            "phase {p}: timed parts from {lo:.1} to {hi:.1}",
        );
    }
}

#[test]
fn the_model_times_each_half_beside_a_steady_fdiv_sibling_within_that_form_s_console_halves() {
    // Phase 1 is the `ppu_issue_mix_switch` probe's steady `fdiv` phase,
    // the form the replay runs.
    use crate::console_ppu_issue_mix_switch::{replay, STEADY_FDIV};
    let w = words_at(CONSOLE);
    let (lo, hi) = range(&parts(&w, STEADY_FDIV));
    let cellgov = replay(STEADY_FDIV, HALF, cellgov_time::Budget::new(1));
    for (h, cellgov) in cellgov.iter().enumerate() {
        assert!(
            (lo..=hi).contains(cellgov),
            "half {h}: CellGov {cellgov:.1} cycles an iteration, the console's halves \
             {lo:.1} to {hi:.1}",
        );
    }
}
