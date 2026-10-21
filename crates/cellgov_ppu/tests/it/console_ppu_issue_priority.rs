//! CellGov's PPU timing against the `ppu_issue_priority` console capture:
//! the thread created after the primary gives way at the shared issue
//! point whatever its LV2 priority.
//!
//! The probe's sibling runs the `ppu_issue_sibling` probe's loop of
//! independent `addi`, the same words, beside three of that probe's
//! chains, created at an LV2 priority below, equal to and above the main
//! thread's. The replay has no LV2 thread records: its sibling is the
//! unit created second.

use crate::console_ppu_issue_sibling_rate::{cellgov_kept, CHAINS_TIMED, SIBLINGS, TOLERANCE};

const CONSOLE: &str = "../../tests/micro/ppu_issue_priority/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 184 when the probe wrote every word.
const DONE: u64 = 0x5050_5052;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The sibling's LV2 priority in phases 1 to 3; the main thread's is
/// 1001.
const PRIORITIES: [u32; 3] = [2000, 1001, 500];

/// The chains the probe times, as indexes into [`CHAINS_TIMED`]: `add`,
/// `fadd` and `ld`.
const CHAINS: [usize; 3] = [0, 3, 4];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 192);
    assert_eq!(words[184], DONE, "the probe wrote every word");
    assert_eq!(words[183], REPS, "five runs a loop");
    for p in 1..=PRIORITIES.len() {
        assert_eq!(words[154 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[161 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[168 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[175 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// The share of its alone rate the sibling of `phase` keeps beside chain
/// `chain` on the console.
fn console_kept(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS.len() * phase + chain;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    rate(w[70 + at], w[105 + at]) / rate(w[140 + 2 * phase], w[141 + 2 * phase])
}

#[test]
fn the_later_thread_keeps_the_model_s_share_of_its_rate_at_every_priority() {
    let w = console_words();
    let (_, addi) = &SIBLINGS[0];
    let mut rows = Vec::new();
    for (c, &chain) in CHAINS.iter().enumerate() {
        let (name, replay) = &CHAINS_TIMED[chain];
        let cellgov = cellgov_kept(replay, addi);
        for (p, priority) in PRIORITIES.iter().enumerate() {
            rows.push((*name, *priority, cellgov, console_kept(&w, p + 1, c)));
        }
    }
    assert_eq!(rows.len(), 9);
    for &(chain, priority, cellgov, console) in &rows {
        // The slowdown: the inverse of the share of its rate it keeps.
        assert!(
            (console / cellgov - 1.0).abs() <= TOLERANCE,
            "beside {chain} at priority {priority}: CellGov keeps {cellgov:.2} of the \
             sibling's rate, the console {console:.2}: {rows:?}",
        );
    }
}

#[test]
fn a_higher_priority_later_thread_still_keeps_little_beside_a_dependent_chain() {
    // Were LV2 priority the order, the sibling at 500 would take the
    // dispatch slots the `fadd` chain cannot use and keep most of its
    // rate; on the console it keeps under a fifth.
    let w = console_words();
    let fadd = 1;
    assert!(console_kept(&w, 3, fadd) < 0.2);
    let (_, addi) = &SIBLINGS[0];
    let (_, chain) = &CHAINS_TIMED[CHAINS[fadd]];
    assert!(cellgov_kept(chain, addi) < 0.2);
}
