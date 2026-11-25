//! CellGov's PPU timing against the `ppu_issue_divide_window_forms`
//! console capture: loops of 0, 2, 4 and 8 independent `addi` and a
//! `bdnz` beside a lower-priority sibling of back-to-back dependent
//! `fsqrt`, `fdivs`, `fsqrts` or `divd`.

use crate::console_ppu_issue_fdiv_window::{cellgov, median, words, ADDI8, TOLERANCE};
use crate::console_ppu_issue_sibling_rate::{same, sibling_loop, COUNT, STOP};
use crate::console_ppu_issue_timing::{Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_divide_window_forms/ps3/cech20-cex-493/observation.json";

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's counted runs of each loop.
const REPS: u64 = 15;

/// Each run's iterations on the console.
const ITERATIONS: u64 = 2048;

/// The `addi` each loop carries.
const K: [usize; 4] = [0, 2, 4, 8];

/// 1.0 as a double.
const ONE: u64 = 0x3ff0_0000_0000_0000;
/// 1.5 as a double.
const ONE_HALF: u64 = 0x3ff8_0000_0000_0000;

const SIB_FSQRT: [u32; 18] = sibling_loop(same(0xfc00_002c), 10, 8);
const SIB_FDIVS: [u32; 18] = sibling_loop(same(0xec00_6024), 10, 8);
const SIB_FSQRTS: [u32; 18] = sibling_loop(same(0xec00_002c), 10, 8);
const SIB_DIVD: [u32; 18] = sibling_loop(same(0x7d4a_43d2), 7, 6);

/// The probe's siblings, phases 1 to 4.
const SIBLINGS: [(&str, Sibling); 4] = [
    (
        "fsqrt",
        Sibling {
            words: &SIB_FSQRT,
            seeds: &[Seed::Gpr(10, STOP), Seed::Gpr(8, COUNT), Seed::Fpr(0, ONE)],
        },
    ),
    (
        "fdivs",
        Sibling {
            words: &SIB_FDIVS,
            seeds: &[
                Seed::Gpr(10, STOP),
                Seed::Gpr(8, COUNT),
                Seed::Fpr(0, ONE_HALF),
                Seed::Fpr(12, ONE),
            ],
        },
    ),
    (
        "fsqrts",
        Sibling {
            words: &SIB_FSQRTS,
            seeds: &[Seed::Gpr(10, STOP), Seed::Gpr(8, COUNT), Seed::Fpr(0, ONE)],
        },
    ),
    (
        "divd",
        Sibling {
            words: &SIB_DIVD,
            seeds: &[
                Seed::Gpr(7, STOP),
                Seed::Gpr(6, COUNT),
                Seed::Gpr(10, 0x7fff_0000),
                Seed::Gpr(8, 1),
            ],
        },
    ),
];

/// The payload, checked whole.
fn console_words() -> Vec<u64> {
    let w = words(CONSOLE, 5120);
    assert_eq!(w[639], 0x5050_4457, "the probe wrote every word");
    assert_eq!((w[631], w[632], w[633], w[634]), (REPS, ITERATIONS, 4, 4));
    for s in 0..SIBLINGS.len() {
        let b = 500 + 10 * s;
        assert_eq!(
            (w[b + 2], w[b + 3], w[b + 4], w[b + 5], w[b + 6]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {}: the sibling's create rc, join rc, exit value, start and sleep rc",
            s + 1,
        );
        for l in 0..K.len() {
            assert_eq!(w[540 + 4 * s + l], REPS, "phase {} loop {l}", s + 1);
        }
    }
    w
}

/// The console's median and CellGov's cycles an iteration of loop `l`
/// beside the sibling of phase `s + 1`.
fn cell(w: &[u64], s: usize, l: usize) -> (f64, f64) {
    let body: Vec<u32> = (0..K[l]).map(|i| ADDI8[i % 8]).collect();
    let console = median(w, 16 + 120 * s + 30 * l);
    (console, cellgov(&body, &[], &SIBLINGS[s].1))
}

/// The console's cycles between two of phase `s + 1`'s sibling's
/// instructions while the main thread slept.
fn period(w: &[u64], s: usize) -> f64 {
    let b = 500 + 10 * s;
    (w[b + 1] * PPU_CYCLES_PER_TB_TICK) as f64 / (w[b] * 32) as f64
}

/// The index in [`SIBLINGS`] of the sibling named `name`.
fn phase(name: &str) -> usize {
    SIBLINGS
        .iter()
        .position(|(n, _)| *n == name)
        .expect("a probe sibling")
}

/// The index in [`K`] of the loop of 8 `addi`: five dispatch groups,
/// which the divide window spaces 6.69 sibling periods an iteration.
const LOOP_8: usize = 3;

/// The sibling periods under which a loop of five dispatch groups keeps
/// no divide window.
const NO_WINDOW_PERIODS: f64 = 3.0;

/// The cells beside `fdivs` and `fsqrts` the model times outside
/// [`TOLERANCE`], as (sibling, `addi`). The console's runs beside those
/// siblings spread from under one sibling period an iteration to several.
/// The spread follows no order of the loop's size, and the model has no
/// rule for it.
const RESIDUALS: [(&str, usize); 7] = [
    ("fdivs", 0),
    ("fdivs", 2),
    ("fdivs", 4),
    ("fdivs", 8),
    ("fsqrts", 0),
    ("fsqrts", 2),
    ("fsqrts", 4),
];

#[test]
fn beside_back_to_back_fsqrt_each_addi_loop_runs_near_the_console_s_median() {
    let w = console_words();
    let s = phase("fsqrt");
    for (l, k) in K.iter().enumerate() {
        let (console, cellgov) = cell(&w, s, l);
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "{k} addi: CellGov {cellgov:.1} cycles an iteration, the console's median {console:.1}",
        );
    }
}

#[test]
fn beside_back_to_back_single_precision_divides_the_loop_of_8_addi_keeps_no_window() {
    // Beside `fsqrt` the loop of 8 `addi` takes about 6.7 sibling periods;
    // beside `fdivs` and `fsqrts` it takes under two on the console.
    let w = console_words();
    let s = phase("fsqrt");
    let (console, _) = cell(&w, s, LOOP_8);
    assert!(
        console > NO_WINDOW_PERIODS * period(&w, s),
        "fsqrt: the console's median {console:.1}, its period {:.1}",
        period(&w, s),
    );
    for name in ["fdivs", "fsqrts"] {
        let s = phase(name);
        let (console, cellgov) = cell(&w, s, LOOP_8);
        let most = NO_WINDOW_PERIODS * period(&w, s);
        assert!(
            console < most && cellgov < most,
            "{name}: CellGov {cellgov:.1} cycles an iteration, the console's median {console:.1}, \
             {NO_WINDOW_PERIODS} periods {most:.1}",
        );
    }
}

#[test]
fn beside_single_precision_divides_the_model_misses_only_the_named_cells() {
    let w = console_words();
    for name in ["fdivs", "fsqrts"] {
        let s = phase(name);
        for (l, &k) in K.iter().enumerate() {
            let (console, cellgov) = cell(&w, s, l);
            let near = (cellgov - console).abs() / console <= TOLERANCE;
            assert_eq!(
                near,
                !RESIDUALS.contains(&(name, k)),
                "{k} addi beside {name}: CellGov {cellgov:.1} cycles an iteration, the console's \
                 median {console:.1}",
            );
        }
    }
}

#[test]
fn beside_back_to_back_divd_the_console_keeps_the_window_and_the_model_does_not() {
    // An accepted gap. The console spaces the loop of 8 `addi` 6.69 `divd`
    // periods an iteration in most runs, as beside `fdiv`. In the longer
    // runs of `ppu_issue_loop_shape` beside the same sibling the loop keeps
    // less than the divide window, so the model opens no window beside
    // `divd`.
    let w = console_words();
    let s = phase("divd");
    let (console, cellgov) = cell(&w, s, LOOP_8);
    let most = NO_WINDOW_PERIODS * period(&w, s);
    assert!(
        console > most && cellgov < most,
        "divd: CellGov {cellgov:.1} cycles an iteration, the console's median {console:.1}, \
         {NO_WINDOW_PERIODS} periods {most:.1}",
    );
}
