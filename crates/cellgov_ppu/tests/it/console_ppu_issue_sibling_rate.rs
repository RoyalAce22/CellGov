//! CellGov's PPU timing against the `ppu_issue_sibling` console capture:
//! what each timed chain costs the sibling thread beside it.
//!
//! The probe's sibling runs one instruction class in a loop of 32
//! instructions that loads a stop flag and counts its rounds in memory,
//! with no time-base read. The capture holds its round count while the
//! main thread sleeps and again while each of five chains runs. The
//! replay runs the same loop words as the sibling.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay, replay_measured, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_sibling/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 184 when the probe wrote every word.
const DONE: u64 = 0x5050_5342;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The chains a phase times.
pub(crate) const CHAINS: usize = 5;

/// The siblings, phases 1 to 6.
pub(crate) const PHASES: usize = 6;

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
    for p in 1..=PHASES {
        assert_eq!(words[154 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[161 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[168 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[175 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// The share of its alone rate the sibling of `phase` keeps beside
/// `chain` on the console.
fn console_kept(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    rate(w[70 + at], w[105 + at]) / rate(w[140 + 2 * phase], w[141 + 2 * phase])
}

/// CellGov's cycles a round of the sibling's loop with no thread beside
/// it: the loop runs as the replay's own, its closing branch replaced by
/// a count down of r21.
fn cellgov_alone(sibling: &Sibling) -> f64 {
    let n = sibling.words.len();
    let mut words: Vec<u32> = sibling.words[..n - 1].to_vec();
    // addi r21,r21,-1; cmpdi r21,0; bne back to the head.
    words.push(0x3ab5_ffff);
    words.push(0x2c35_0000);
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4082_0000 | back);
    let mut seeds = sibling.seeds.to_vec();
    seeds.push(Seed::Counter(21));
    let r = Replay {
        loop_index: 0,
        words: Box::leak(words.into_boxed_slice()),
        seeds: Box::leak(seeds.into_boxed_slice()),
    };
    replay(&r, 0).expect("the sibling's loop replays alone")
}

/// The share of its alone rate CellGov's sibling keeps beside `chain`.
pub(crate) fn cellgov_kept(chain: &Replay, sibling: &Sibling) -> f64 {
    kept_of(cellgov_alone(sibling), chain, sibling)
}

/// The same share, with `alone` the sibling's cycles a round alone.
fn kept_of(alone: f64, chain: &Replay, sibling: &Sibling) -> f64 {
    let beside = replay_measured(chain, 1, sibling)
        .and_then(|m| m.sibling_cycles)
        .expect("the sibling ran beside the chain");
    alone / beside
}

/// The stop flag and the round count the replayed sibling reads and
/// writes: zero, so it never stops.
pub(crate) const STOP: u64 = 0x6000;
pub(crate) const COUNT: u64 = 0x6080;

/// The sibling's loop: `li r14,4; mtctr r14`, `body` to a `bdnz`, then
/// the stop-flag load, the round count's increment and the branch back,
/// with the stop flag at `ra_stop` and the count at `ra_count`.
pub(crate) const fn sibling_loop(body: [u32; 8], ra_stop: u32, ra_count: u32) -> [u32; 18] {
    let mut w = [0u32; 18];
    w[0] = 0x39c0_0004;
    w[1] = 0x7dc9_03a6;
    let mut i = 0;
    while i < 8 {
        w[2 + i] = body[i];
        i += 1;
    }
    w[10] = 0x4200_ffe0;
    w[11] = 0x8000_0000 | (ra_stop << 16);
    w[12] = 0x2c00_0000;
    w[13] = 0x4082_0014;
    w[14] = 0xe920_0000 | (ra_count << 16);
    w[15] = 0x3929_0001;
    w[16] = 0xf920_0000 | (ra_count << 16);
    w[17] = 0x4bff_ffbc;
    w
}

pub(crate) const fn same(word: u32) -> [u32; 8] {
    [word; 8]
}

const SIB_ADDI: [u32; 18] = sibling_loop(
    [
        0x3a31_0001,
        0x3a52_0001,
        0x3a73_0001,
        0x3a94_0001,
        0x3a31_0001,
        0x3a52_0001,
        0x3a73_0001,
        0x3a94_0001,
    ],
    10,
    8,
);
const SIB_DIVD: [u32; 18] = sibling_loop(same(0x7def_83d2), 10, 8);
const SIB_MULLD: [u32; 18] = sibling_loop(same(0x7def_81d2), 10, 8);
const SIB_FDIV: [u32; 18] = sibling_loop(same(0xfc00_6024), 10, 8);
const SIB_FADD: [u32; 18] = sibling_loop(same(0xfc00_602a), 10, 8);
const SIB_LHA: [u32; 18] = sibling_loop(
    [
        0xaa2a_0000,
        0xaa4a_0002,
        0xaa6a_0004,
        0xaa8a_0006,
        0xaa2a_0000,
        0xaa4a_0002,
        0xaa6a_0004,
        0xaa8a_0006,
    ],
    8,
    7,
);

/// 1.0 and 1.5 as doubles.
const ONE: u64 = 0x3ff0_0000_0000_0000;
const ONE_HALF: u64 = 0x3ff8_0000_0000_0000;

const GPR_SEEDS: &[Seed] = &[
    Seed::Gpr(10, STOP),
    Seed::Gpr(8, COUNT),
    Seed::Gpr(15, 0x7fff_0000),
    Seed::Gpr(16, 1),
];

/// The probe's siblings, phases 1 to 6.
pub(crate) const SIBLINGS: [(&str, Sibling); PHASES] = [
    (
        "addi",
        Sibling {
            words: &SIB_ADDI,
            seeds: GPR_SEEDS,
        },
    ),
    (
        "divd",
        Sibling {
            words: &SIB_DIVD,
            seeds: GPR_SEEDS,
        },
    ),
    (
        "mulld",
        Sibling {
            words: &SIB_MULLD,
            seeds: GPR_SEEDS,
        },
    ),
    (
        "fdiv",
        Sibling {
            words: &SIB_FDIV,
            seeds: &[
                Seed::Gpr(10, STOP),
                Seed::Gpr(8, COUNT),
                Seed::Fpr(0, ONE_HALF),
                Seed::Fpr(12, ONE),
            ],
        },
    ),
    (
        "fadd",
        Sibling {
            words: &SIB_FADD,
            seeds: &[
                Seed::Gpr(10, STOP),
                Seed::Gpr(8, COUNT),
                Seed::Fpr(0, ONE),
                Seed::Fpr(12, 0),
            ],
        },
    ),
    (
        "lha",
        Sibling {
            words: &SIB_LHA,
            seeds: &[
                Seed::Gpr(8, STOP),
                Seed::Gpr(7, COUNT),
                Seed::Gpr(10, 0x4080),
            ],
        },
    ),
];

/// A line whose first doubleword points at itself.
const SELF_LINE: u64 = 0x4000;

/// The probe's chains, in its order, and their names.
pub(crate) const CHAINS_TIMED: [(&str, Replay); CHAINS] = [
    (
        "add",
        Replay {
            loop_index: 0,
            words: &eight(0x7d29_7a14),
            seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
        },
    ),
    (
        "mulli",
        Replay {
            loop_index: 1,
            words: &eight(0x1d29_0001),
            seeds: &[Seed::Gpr(9, 7)],
        },
    ),
    (
        "addi",
        Replay {
            loop_index: 2,
            words: &[
                0x3a31_0001,
                0x3a52_0001,
                0x3a73_0001,
                0x3a94_0001,
                0x3a31_0001,
                0x3a52_0001,
                0x3a73_0001,
                0x3a94_0001,
                0x3a31_0001,
                0x3a52_0001,
                0x3a73_0001,
                0x3a94_0001,
                0x3a31_0001,
                0x3a52_0001,
                0x3a73_0001,
                0x3a94_0001,
                0x4200_ffc0,
            ],
            seeds: &[],
        },
    ),
    (
        "fadd",
        Replay {
            loop_index: 3,
            words: &eight(0xfc00_602a),
            seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
        },
    ),
    (
        "ld",
        Replay {
            loop_index: 4,
            words: &eight(0xe929_0000),
            seeds: &[Seed::Ring(9, SELF_LINE, 128, false)],
        },
    ),
];

/// The largest share of the console's value the model may sit from it:
/// of a sibling's cycles a round alone, or of its slowdown beside a
/// chain.
pub(crate) const TOLERANCE: f64 = 0.20;

/// The cells whose slowdown the model times outside [`TOLERANCE`], as
/// (sibling, chain):
///
/// - `fdiv` beside `mulli`, 25% short. On the console every sibling of
///   long instructions keeps the least of its rate beside the `mulli`
///   chain, 0.61 to 0.78, and the `fdiv` sibling 0.73. The model's `fdiv`
///   sibling keeps 0.97 there: the chain issues more often than the
///   sibling, and no stall of a multiply reaches the sibling.
const RESIDUALS: [(&str, &str); 1] = [("fdiv", "mulli")];

/// How far from the console's slowdown a residual may sit.
const RESIDUAL_LIMIT: f64 = 0.35;

#[test]
fn each_sibling_s_loop_alone_runs_near_the_console() {
    let w = console_words();
    for (p, (sibling, s)) in SIBLINGS.iter().enumerate() {
        let at = 140 + 2 * (p + 1);
        let console = (PPU_CYCLES_PER_TB_TICK * w[at + 1]) as f64 / w[at] as f64;
        let cellgov = cellgov_alone(s);
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "{sibling} alone: CellGov {cellgov:.1} cycles a round, the console {console:.1}",
        );
    }
}

#[test]
fn each_sibling_keeps_its_console_share_of_its_rate_beside_each_chain() {
    let w = console_words();
    let mut rows = Vec::new();
    for (p, (sibling, s)) in SIBLINGS.iter().enumerate() {
        let alone = cellgov_alone(s);
        for (c, (chain, r)) in CHAINS_TIMED.iter().enumerate() {
            rows.push((
                *sibling,
                *chain,
                kept_of(alone, r, s),
                console_kept(&w, p + 1, c),
            ));
        }
    }
    assert_eq!(rows.len(), PHASES * CHAINS);
    let mut residuals = 0;
    for &(sibling, chain, cellgov, console) in &rows {
        // The slowdown: the sibling's cycles a round beside the chain over
        // its cycles alone, the inverse of the share it keeps.
        let off = (console / cellgov - 1.0).abs();
        if RESIDUALS.contains(&(sibling, chain)) {
            residuals += 1;
            assert!(
                off > TOLERANCE,
                "{sibling} beside {chain} now keeps near the console's share; drop it from \
                 the residuals: {rows:?}",
            );
            assert!(off <= RESIDUAL_LIMIT, "{sibling} beside {chain}: {rows:?}");
        } else {
            assert!(
                off <= TOLERANCE,
                "{sibling} beside {chain}: CellGov keeps {cellgov:.2} of its rate, the \
                 console {console:.2}: {rows:?}",
            );
        }
    }
    assert_eq!(residuals, RESIDUALS.len(), "every residual was timed");
}

#[test]
fn the_addi_sibling_keeps_less_beside_a_chain_that_issues_less_often() {
    // On the console the `addi` sibling keeps 0.54 of its rate beside
    // dependent `add`, 0.29 beside dependent `ld` and 0.14 beside
    // dependent `fadd`: about one issue to each of the chain's.
    let w = console_words();
    let (_, addi) = &SIBLINGS[0];
    let [(_, add), _, _, (_, fadd), (_, ld)] = &CHAINS_TIMED;
    let console = |c: usize| console_kept(&w, 1, c);
    assert!(console(3) < console(4) && console(4) < console(0));
    let alone = cellgov_alone(addi);
    let (a, l, f) = (
        kept_of(alone, add, addi),
        kept_of(alone, ld, addi),
        kept_of(alone, fadd, addi),
    );
    assert!(
        f < l && l < a,
        "CellGov beside add {a:.2}, ld {l:.2}, fadd {f:.2}"
    );
}
