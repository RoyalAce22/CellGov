//! CellGov's PPU timing against the `ppu_issue_layout` console capture.
//!
//! The probe times one chain, 8 dependent `add` and a `bdnz`, at eight
//! 16-byte offsets into a 128-byte instruction-cache line. It runs alone
//! and beside a sibling thread of microcoded `lha` or dependent `mulld`,
//! each with its loop at two line offsets, dependent `divd` or dependent
//! `fdiv`. Each offset is a fetch-block boundary, so the model times
//! every offset alike.
//!
//! Where the sibling holds its hardware thread through a cell, the
//! chain's cost moves little with the chain's offset: beside `lha` at
//! +0x00 each `add` waits 9.3 to 10.2 cycles at all seven held offsets.
//! The sibling's loop at +0x40 lowers it, to 7.4 and 8.7 beside `lha`
//! and 5.6 to 6.3 beside `mulld`, against 6.3 to 6.7 at +0x00. Where the
//! operating system takes the sibling off its thread for part of the
//! cell, the chain runs nearer its alone time. CellGov keeps a runnable
//! sibling on its thread, so the replay holds only the first kind of
//! cell to the console.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_layout/ps3/cech20-cex-493/cgov_frame.bin";

/// Each timed loop's iterations on the console, from the probe's header.
const ITERATIONS: u64 = 1 << 16;

/// The probe's repetitions of each loop.
const REPS: u64 = 5;

/// The payload's word 268 when the probe wrote every word.
const DONE: u64 = 0x5050_4c59;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The line offsets a phase times the chain at.
const OFFSETS: usize = 8;

/// The siblings, phases 1 to 6.
const PHASES: usize = 6;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 272);
    assert_eq!(words[268], DONE, "the probe wrote every word");
    assert_eq!(words[267], REPS, "five runs a loop");
    for p in 1..=PHASES {
        assert_eq!(words[238 + p], 0, "phase {p}'s sibling was created");
        assert_eq!(words[245 + p], 0, "phase {p}'s sibling joined");
        assert_eq!(words[252 + p], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[259 + p], 1, "phase {p}'s sibling ran");
    }
    words
}

/// One cell of the capture.
#[derive(Debug, Clone, Copy)]
struct Cell {
    fastest: f64,
    slowest: f64,
    mean: f64,
    /// The sibling's rate beside the cell as a share of its rate alone.
    present: f64,
}

/// Loop `OFFSETS * phase + offset`, in cycles an iteration.
fn cell(w: &[u64], phase: usize, offset: usize) -> Cell {
    let at = OFFSETS * phase + offset;
    let per = |tb: u64, iterations: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    let rate = |rounds: u64, ticks: u64| rounds as f64 / ticks as f64;
    let present = if phase == 0 {
        0.0
    } else {
        rate(w[112 + at], w[168 + at]) / rate(w[224 + 2 * phase], w[225 + 2 * phase])
    };
    Cell {
        fastest: per(w[2 * at], ITERATIONS),
        slowest: per(w[2 * at + 1], ITERATIONS),
        mean: per(w[168 + at], REPS * ITERATIONS),
        present,
    }
}

/// A sibling that ran beside a cell at this share of its alone rate or
/// more, with the cell's fastest run at least [`NARROW`] of its slowest,
/// held its thread through it.
const HELD: f64 = 0.8;

/// The share of a cell's slowest run its fastest reaches when no run
/// lost the sibling for part of its time. In this capture each cell at
/// [`HELD`] or more has its fastest run at 0.72 or more of its slowest,
/// or at 0.64 or less.
const NARROW: f64 = 0.7;

/// Whether the sibling held its thread through every run of `c`.
fn held(c: &Cell) -> bool {
    c.present >= HELD && c.fastest >= NARROW * c.slowest
}

/// The held cells the model times outside both the console's spread and
/// [`TOLERANCE`], as (sibling, offset): the chain beside `lha` whose loop
/// sits at +0x40 waits 7.4 and 8.7 cycles an `add` there, against 9.3 to
/// 10.2 at every held offset beside the same loop at +0x00 and about 11
/// in the model.
const RESIDUALS: [(&str, usize); 2] = [("lha at +0x40", 4), ("lha at +0x40", 7)];

/// How far from the console's mean a residual may sit.
const RESIDUAL_LIMIT: f64 = 0.45;

/// A sibling below this share was off its thread for much of the cell.
const OFF: f64 = 0.6;

/// The largest share of a cell's mean console time the model may sit
/// from it, for a cell whose console runs spread over less.
const TOLERANCE: f64 = 0.20;

const CHAIN: Replay = Replay {
    loop_index: 0,
    words: &eight(0x7d29_7a14),
    seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
};

const SIB_LHA: [u32; 9] = [
    0xaa26_0000,
    0xaa46_0002,
    0xaa66_0004,
    0xaa86_0006,
    0xaa26_0000,
    0xaa46_0002,
    0xaa66_0004,
    0xaa86_0006,
    crate::console_ppu_issue_timing::BDNZ_8,
];
const SIB_MULLD: [u32; 9] = eight(0x7def_81d2);
const SIB_DIVD: [u32; 9] = eight(0x7def_83d2);
const SIB_FDIV: [u32; 9] = eight(0xfc00_6024);

const INT_SEEDS: &[Seed] = &[Seed::Gpr(15, 0x7fff_0000), Seed::Gpr(16, 1)];
const LHA: Sibling = Sibling {
    words: &SIB_LHA,
    seeds: &[Seed::Gpr(6, 0x4080)],
};
const MULLD: Sibling = Sibling {
    words: &SIB_MULLD,
    seeds: INT_SEEDS,
};

/// The probe's siblings, phases 1 to 6, by name and loop offset.
const SIBLINGS: [(&str, Sibling); PHASES] = [
    ("lha at +0x00", LHA),
    ("lha at +0x40", LHA),
    ("mulld at +0x00", MULLD),
    ("mulld at +0x40", MULLD),
    (
        "divd",
        Sibling {
            words: &SIB_DIVD,
            seeds: INT_SEEDS,
        },
    ),
    (
        "fdiv",
        Sibling {
            words: &SIB_FDIV,
            seeds: &[
                Seed::Fpr(0, 0x3ff8_0000_0000_0000),
                Seed::Fpr(12, 0x3ff0_0000_0000_0000),
            ],
        },
    ),
];

#[test]
fn beside_a_sibling_that_holds_its_thread_the_issue_model_times_every_offset_near_the_console() {
    let w = console_words();
    let (mut checked, mut residuals) = (0, 0);
    for (p, (name, sibling)) in SIBLINGS.iter().enumerate() {
        let cellgov = replay_beside(&CHAIN, 1, sibling).expect("the chain replays");
        for offset in 0..OFFSETS {
            let c = cell(&w, p + 1, offset);
            if !held(&c) {
                continue;
            }
            checked += 1;
            let off = (cellgov - c.mean).abs() / c.mean;
            let near = (c.fastest..=c.slowest).contains(&cellgov) || off <= TOLERANCE;
            if RESIDUALS.contains(&(name, offset)) {
                residuals += 1;
                assert!(
                    !near && off <= RESIDUAL_LIMIT,
                    "beside {name}, offset {:#x}: CellGov {cellgov:.1}, the console {c:?}",
                    16 * offset,
                );
            } else {
                assert!(
                    near,
                    "beside {name}, offset {:#x}: CellGov {cellgov:.1} cycles an iteration, \
                     the console {c:?}",
                    16 * offset,
                );
            }
        }
    }
    assert_eq!(residuals, RESIDUALS.len(), "every residual was a held cell");
    assert!(checked >= 25, "only {checked} of 48 cells held the sibling");
}

#[test]
fn a_sibling_off_its_thread_costs_the_chain_less_than_one_that_holds_it() {
    // For each sibling, every cell it was off its thread for loses less
    // than every cell it held its thread through, and the model, whose
    // sibling holds its thread, times each such cell slower than the
    // console.
    let w = console_words();
    let alone: Vec<f64> = (0..OFFSETS).map(|o| cell(&w, 0, o).mean).collect();
    let mut off_cells = 0;
    for (p, (name, sibling)) in SIBLINGS.iter().enumerate() {
        let cellgov = replay_beside(&CHAIN, 1, sibling).expect("the chain replays");
        for o in 0..OFFSETS {
            let c = cell(&w, p + 1, o);
            if c.present < OFF {
                assert!(
                    cellgov > c.mean,
                    "beside {name}: CellGov {cellgov:.1}, {c:?}"
                );
            }
        }
        let cells: Vec<(f64, f64)> = (0..OFFSETS)
            .map(|o| {
                let c = cell(&w, p + 1, o);
                (c.present, c.mean - alone[o])
            })
            .collect();
        let least_held = cells
            .iter()
            .filter(|c| c.0 >= HELD)
            .map(|c| c.1)
            .fold(f64::INFINITY, f64::min);
        for &(present, lost) in cells.iter().filter(|c| c.0 < OFF) {
            off_cells += 1;
            assert!(
                lost < least_held,
                "beside {name}: a cell at {present:.2} of the sibling's rate lost {lost:.1}, \
                 the least a held cell lost {least_held:.1}"
            );
        }
    }
    assert!(off_cells > 0, "no cell took the sibling off its thread");
}

#[test]
fn the_chain_alone_times_as_the_model_at_every_offset() {
    let w = console_words();
    let cellgov = replay(&CHAIN, 0).expect("the chain replays");
    for offset in 0..OFFSETS {
        let c = cell(&w, 0, offset);
        assert!(
            cellgov <= c.fastest && (c.fastest - cellgov) / c.fastest <= 0.13,
            "offset {:#x}: CellGov {cellgov:.2}, the console {c:?}",
            16 * offset,
        );
    }
}
