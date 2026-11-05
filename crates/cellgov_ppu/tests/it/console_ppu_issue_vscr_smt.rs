//! CellGov's PPU timing against the `ppu_issue_vscr_smt` console capture:
//! whether one thread's `mtvscr` stalls the sibling thread's work.
//!
//! The probe's sibling runs one loop of 32 instructions that loads a stop
//! flag and counts its rounds in memory, with no time-base read. Five
//! chains run on the main thread alone and beside each sibling.

use crate::console_ppu_issue_sibling_rate::{cellgov_kept, sibling_loop, COUNT, STOP, TOLERANCE};
use crate::console_ppu_issue_timing::{replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_smt/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 132 when the probe wrote every word.
const DONE: u64 = 0x5050_5753;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The chains a phase times, and the siblings, phases 1 to 4.
const CHAINS: usize = 5;
const PHASES: usize = 4;

/// log2 of each chain's iterations on the console.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 136);
    assert_eq!(words[132], DONE, "the probe wrote every word");
    assert_eq!(words[131], 5, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[110 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[115 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[120 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[125 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// Chain `chain`'s mean run in `phase`, in core cycles an iteration.
///
/// Beside a sibling of `mtvscr` a run's cycles move with how the two
/// threads line up: the chain of an `mtvscr` and four `vaddubm` runs 72
/// to 210 cycles an iteration. The model gives a rate, which the mean of
/// the five runs measures and the fastest run does not.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[75 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / 5.0 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `addi rk, rk, 1`.
const fn addi(k: u32) -> u32 {
    0x3800_0001 | (k << 21) | (k << 16)
}
/// `fadd f1, f1, f2`.
const FADD: u32 = 0xfc21_102a;
/// 1.0 as a double.
const ONE: u64 = 0x3ff0_0000_0000_0000;

/// The compiled sibling holds the stop flag's address in r10 and the
/// round count's in r8.
const SIBLING_SEEDS: &[Seed] = &[Seed::Gpr(10, STOP), Seed::Gpr(8, COUNT)];

/// Each phase's sibling, phases 1 to 4.
fn siblings() -> [Sibling; PHASES] {
    let leak = |w: [u32; 18]| -> &'static [u32] { Box::leak(Box::new(w)) };
    let v = |k| vadd(k);
    [
        Sibling {
            words: leak(sibling_loop(
                [v(1), v(2), v(3), v(4), v(1), v(2), v(3), v(4)],
                10,
                8,
            )),
            seeds: SIBLING_SEEDS,
        },
        Sibling {
            words: leak(sibling_loop([MTVSCR; 8], 10, 8)),
            seeds: SIBLING_SEEDS,
        },
        Sibling {
            words: leak(sibling_loop(
                [MTVSCR, v(1), v(2), v(3), v(4), v(5), v(6), v(7)],
                10,
                8,
            )),
            seeds: SIBLING_SEEDS,
        },
        Sibling {
            words: leak(sibling_loop(
                [
                    addi(17),
                    addi(18),
                    addi(19),
                    addi(20),
                    addi(17),
                    addi(18),
                    addi(19),
                    addi(20),
                ],
                10,
                8,
            )),
            seeds: SIBLING_SEEDS,
        },
    ]
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Each chain, in the probe's order.
fn chains() -> [Replay; CHAINS] {
    let vadd4: Vec<u32> = (1..=4).map(vadd).collect();
    let vadd8: Vec<u32> = (1..=8).map(vadd).collect();
    let addi16: Vec<u32> = (0..16).map(|i| addi(17 + i % 4)).collect();
    let chain = |k: usize, body: Vec<u32>, seeds: &'static [Seed]| Replay {
        loop_index: k,
        words: looped(&body),
        seeds,
    };
    [
        chain(0, vadd8, &[]),
        chain(1, vec![MTVSCR], &[]),
        chain(2, [&[MTVSCR][..], &vadd4].concat(), &[]),
        chain(3, addi16, &[]),
        chain(4, vec![FADD; 8], &[Seed::Fpr(1, ONE), Seed::Fpr(2, 0)]),
    ]
}

/// CellGov's cycles an iteration of chain `chain` beside the sibling of
/// `phase`.
fn cellgov_beside(phase: usize, chain: usize) -> f64 {
    replay_beside(&chains()[chain], 1, &siblings()[phase - 1]).expect("the chain replays")
}

/// The share of the console's mean run a held cell may lie on either side
/// of it: the chain of an `mtvscr` and four `vaddubm` beside the `mtvscr`
/// sibling lies 22% above.
const BAND: f64 = 0.25;

#[test]
fn a_sibling_of_mtvscr_stalls_each_chain_beside_it_within_the_band_of_the_mean_run() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..CHAINS)
        .map(|c| (c, cellgov_beside(2, c), console_mean(&w, 2, c)))
        .collect();
    for &(c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's mean \
             {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn a_sibling_with_no_mtvscr_adds_no_mtvscr_stall_to_any_chain_within_the_band_of_the_mean_run() {
    let w = console_words();
    // Phase 1's sibling is independent `vaddubm`, phase 4's independent
    // `addi`.
    let rows: Vec<(usize, usize, f64, f64)> = [1, 4]
        .iter()
        .flat_map(|&p| (0..CHAINS).map(move |c| (p, c)))
        .map(|(p, c)| (p, c, cellgov_beside(p, c), console_mean(&w, p, c)))
        .collect();
    assert_eq!(rows.len(), 2 * CHAINS);
    for &(p, c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             mean {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

/// The chains the model does not yet time beside a sibling of an `mtvscr`
/// and seven `vaddubm` (phase 3): the independent `vaddubm` and `addi`
/// chains. On the console they take about 1.4 and 2.1 times the model's
/// cycles.
///
/// The test fails once a chain comes inside [`BAND`] of the console, so
/// the chain moves into the held set.
const FENCED_SIBLING_DIFFERENCES: [usize; 2] = [0, 3];

#[test]
fn the_mtvscr_and_fadd_chains_beside_fenced_vector_work_lie_within_the_band_of_the_mean_run() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = (0..CHAINS)
        .filter(|c| !FENCED_SIBLING_DIFFERENCES.contains(c))
        .map(|c| (c, cellgov_beside(3, c), console_mean(&w, 3, c)))
        .collect();
    assert_eq!(rows.len(), CHAINS - FENCED_SIBLING_DIFFERENCES.len());
    for &(c, cellgov, console) in &rows {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase 3 chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             mean {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn the_chains_beside_fenced_vector_work_are_known_differences() {
    let w = console_words();
    for c in FENCED_SIBLING_DIFFERENCES {
        let cell = (cellgov_beside(3, c), console_mean(&w, 3, c));
        assert!(
            cell.0 < (1.0 - BAND) * cell.1,
            "phase 3 chain {c}: CellGov {:.2}, the console's mean {:.2}",
            cell.0,
            cell.1
        );
    }
}

/// The share of its rate the sibling of `phase` keeps beside `chain` on
/// the console: its rounds a tick while the chain's five runs ran, over
/// its rounds a tick while the main thread slept.
fn console_kept(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    rate(w[50 + at], w[75 + at]) / rate(w[100 + 2 * phase], w[101 + 2 * phase])
}

/// The cells, as (phase, chain), whose sibling keeps more of its rate in
/// the model than on the console by more than [`TOLERANCE`] of its
/// slowdown:
///
/// - the `mtvscr` sibling (phase 2) beside the two `mtvscr` chains and the
///   `fadd` chain: on the console it keeps 0.40 to 0.70, in the model 0.78
///   to 0.93;
/// - the `vaddubm` sibling and the sibling of an `mtvscr` and seven
///   `vaddubm` beside the `mtvscr` chain: on the console the sibling
///   loses about a third more of its rate than in the model.
///
/// The test fails once a difference closes, so the cell moves into the
/// held set.
const KEPT_DIFFERENCES: [(usize, usize); 5] = [(1, 1), (2, 1), (2, 2), (2, 4), (3, 1)];

#[test]
fn each_sibling_keeps_its_console_share_of_its_rate_beside_each_chain() {
    let w = console_words();
    let chains = chains();
    let siblings = siblings();
    let rows: Vec<(usize, usize, f64, f64)> = (1..=PHASES)
        .flat_map(|p| (0..CHAINS).map(move |c| (p, c)))
        .map(|(p, c)| {
            let cellgov = cellgov_kept(&chains[c], &siblings[p - 1]);
            (p, c, cellgov, console_kept(&w, p, c))
        })
        .collect();
    assert_eq!(rows.len(), PHASES * CHAINS);
    let mut differences = 0;
    for &(p, c, cellgov, console) in &rows {
        // The slowdown: the sibling's cycles a round beside the chain over
        // its cycles alone, the inverse of the share it keeps.
        let off = (console / cellgov - 1.0).abs();
        if KEPT_DIFFERENCES.contains(&(p, c)) {
            differences += 1;
            assert!(
                off > TOLERANCE,
                "phase {p} chain {c}: the sibling now keeps near the console's share; \
                 move the cell into the held set: {rows:?}",
            );
        } else {
            assert!(
                off <= TOLERANCE,
                "phase {p} chain {c}: CellGov's sibling keeps {cellgov:.3} of its rate, the \
                 console's {console:.3}: {rows:?}",
            );
        }
    }
    assert_eq!(
        differences,
        KEPT_DIFFERENCES.len(),
        "every difference was timed"
    );
}
