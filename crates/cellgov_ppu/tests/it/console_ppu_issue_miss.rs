//! CellGov's PPU timing against the `ppu_issue_miss` console capture: a
//! pointer chase through a shuffled 8 MB ring, one L2 miss a load, with
//! k independent `addi` after each load, alone and beside a sibling of
//! dependent `fdiv` or `divd`.
//!
//! The `divd` phase is not held: its means do not rise with k (1119,
//! 924, 752, 1159 cycles a load) and its runs spread wider than any trend
//! (629 to 1615 at k = 0).

use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_miss/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 72 when the probe wrote every word.
const DONE: u64 = 0x5050_534D;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// Each timed loop's iterations on the console.
const ITERATIONS: u64 = 4096;

/// The `addi` after each load, chains 0 to 3.
const KS: [usize; 4] = [0, 8, 16, 32];

/// Where the replay builds its ring.
const RING: u64 = 0x10_0000;

/// The ring's size, as the probe's.
const RING_BYTES: u64 = 8 * 1024 * 1024;

/// `ld r9,0(r9)`, then `k` `addi` over r17 to r20, then a `bdnz` back.
fn chase(k: usize) -> Replay {
    let addi = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
    let mut words = vec![0xe929_0000u32];
    words.extend((0..k).map(|i| addi[i % 4]));
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: Box::leak(vec![Seed::Ring(9, RING, RING_BYTES, true)].into_boxed_slice()),
    }
}

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 80);
    assert_eq!(words[72], DONE, "the probe wrote every word");
    assert_eq!(words[71], REPS, "five runs a loop");
    for p in 1..=2 {
        assert_eq!(words[54 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[57 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[60 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[63 + p], 1, "phase {p}'s sibling ran");
        assert_eq!(words[66 + p], 0, "phase {p}'s sleep returned");
    }
    words
}

/// Chain `chain`'s mean cycles an iteration in phase `phase` on the
/// console.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = 4 * phase + chain;
    (w[36 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / (REPS * ITERATIONS) as f64
}

fn sibling(name: &str) -> &'static Sibling {
    &SIBLINGS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a ppu_issue_sibling sibling")
        .1
}

/// The largest share of the console's mean the model may sit from it
/// alone.
const TOLERANCE: f64 = 0.20;

/// The `fdiv` phase.
const FDIV: usize = 1;

#[test]
fn alone_the_instructions_after_a_miss_hide_under_it() {
    // Alone each load takes 432 to 457 cycles whatever k: the `addi` issue
    // while the miss is outstanding.
    let w = console_words();
    for (c, &k) in KS.iter().enumerate() {
        let console = console_mean(&w, 0, c);
        assert!(
            (console - console_mean(&w, 0, 0)).abs() / console < 0.1,
            "k = {k}"
        );
        let cellgov = replay(&chase(k), 0).expect("the chase replays");
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "k = {k}: CellGov {cellgov:.0} cycles a load, the console {console:.0}",
        );
    }
}

#[test]
fn beside_a_stalling_sibling_a_miss_no_longer_hides_the_instructions_after_it() {
    // Beside an `fdiv` sibling that holds its thread, each `addi` after a
    // load adds about 40 cycles: 819 cycles a load at k = 0, 2131 at
    // k = 32. The miss's result waits out every stall the instructions
    // after it take, as the model delays a pending result by each stall.
    let w = console_words();
    let fdiv = sibling("fdiv");
    let slope = |t: &dyn Fn(usize) -> f64| (t(3) - t(0)) / KS[3] as f64;
    let console = slope(&|c| console_mean(&w, FDIV, c));
    let cellgov = slope(&|c| replay_beside(&chase(KS[c]), 1, fdiv).expect("the chase replays"));
    assert!(console > 20.0, "the console's cycles an addi: {console:.1}");
    assert!(cellgov > 10.0, "CellGov's cycles an addi: {cellgov:.1}");
}

#[test]
fn beside_a_stalling_sibling_the_model_times_a_miss_short_of_the_console() {
    // An accepted gap. The console's stall beside this `fdiv` sibling, an
    // assembly loop with no time-base read, is about 40 cycles an
    // instruction, and the miss itself takes about 300 more than alone.
    // The model's stall is the 13 the probes with a C-loop sibling hold.
    let w = console_words();
    let fdiv = sibling("fdiv");
    for (c, &k) in KS.iter().enumerate() {
        let at = 4 * FDIV + c;
        let alone_rate = w[48 + 2 * FDIV] as f64 / w[49 + 2 * FDIV] as f64;
        let kept = (w[24 + at] as f64 / w[36 + at] as f64) / alone_rate;
        assert!(
            kept > 0.9,
            "k = {k}: the fdiv sibling kept {kept:.2} of its rate"
        );
        let console = console_mean(&w, FDIV, c);
        let cellgov = replay_beside(&chase(k), 1, fdiv).expect("the chase replays");
        // The bound sits below the slowest of the frame's five runs at
        // k = 32 (0.34 of it).
        assert!(
            cellgov < console && cellgov > 0.3 * console,
            "k = {k}: CellGov {cellgov:.0} cycles a load, the console {console:.0}",
        );
    }
}
