//! CellGov's PPU timing against the `ppu_issue_viq_spaced` console
//! capture: sixteen independent `addi` timed beside a sibling whose every
//! round runs N dependent `fdiv`, K independent `addi` after each, and
//! then 256 independent `addi`.
//!
//! The replays run the probe's two threads through the runtime at the
//! runtime's own step.

use crate::console_ppu_issue_viq_depth::replay;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_viq_spaced/ps3/cech20-cex-493/observation.json";

/// The payload's word 822 when the probe wrote every word.
const DONE: u64 = 0x5050_5653;

/// The runs that count in each phase.
const REPS: usize = 15;

/// The first word of the phases' records.
const META: usize = 630;

/// The `addi` after each `fdiv`, by row.
const KS: [usize; 4] = [0, 1, 2, 4];

/// The dependent `fdiv` a round, by column.
const NS: [usize; 5] = [5, 6, 7, 8, 10];

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 6584);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[822], DONE, "the probe wrote every word");
    assert_eq!(w[820], REPS as u64, "fifteen runs that count a phase");
    for p in 0..=KS.len() * NS.len() {
        let base = META + 9 * p;
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

/// The timed loop's iterations a run.
fn iterations(w: &[u64]) -> u64 {
    w[821]
}

/// The console's median run beside the sibling of `n` divides with `k`
/// `addi` after each, in cycles an iteration.
fn console_median(w: &[u64], n: usize, k: usize) -> f64 {
    let row = KS.iter().position(|&x| x == k).expect("a timed K");
    let col = NS.iter().position(|&x| x == n).expect("a timed N");
    let p = 1 + NS.len() * row + col;
    let mut runs: Vec<f64> = (0..REPS)
        .map(|r| (w[2 * (REPS * p + r)] * PPU_CYCLES_PER_TB_TICK) as f64 / iterations(w) as f64)
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[REPS / 2]
}

#[test]
fn on_the_console_an_addi_after_each_divide_leaves_room_for_fewer_divides() {
    let w = console_words();
    let flat = console_median(&w, 5, 0);
    assert!(
        (console_median(&w, 7, 0) - flat).abs() / flat < 0.02,
        "back to back: 7 divides {:.2}, 5 divides {flat:.2}",
        console_median(&w, 7, 0)
    );
    let spaced = console_median(&w, 5, 1);
    assert!(
        spaced > 1.1 * flat,
        "5 divides: an addi after each {spaced:.2}, back to back {flat:.2}"
    );
    for pair in NS.windows(2) {
        let (fewer, more) = (
            console_median(&w, pair[0], 1),
            console_median(&w, pair[1], 1),
        );
        assert!(
            more > fewer + 2.0,
            "an addi after each: {} divides {more:.2}, {} divides {fewer:.2}",
            pair[1],
            pair[0]
        );
    }
}

/// The share of the console's median the model may lie from it.
const BAND: f64 = 0.07;

/// Whether the model times the loop one divide's stall slower than the
/// console, for `n` divides with `k` `addi` after each.
fn stalls_a_divide_early(n: usize, k: usize) -> bool {
    k == 2 && n >= 6
}

#[test]
fn beside_a_sibling_of_spaced_divides_the_model_times_the_loop_near_the_console() {
    let w = console_words();
    let iterations = iterations(&w);
    let cells: Vec<(usize, usize)> = KS
        .iter()
        .flat_map(|&k| NS.iter().map(move |&n| (n, k)))
        .filter(|&(n, k)| !stalls_a_divide_early(n, k))
        .collect();
    // A shorter replay or warm-up moves a cell's cycles, since each
    // replay times how the loop lines up with the sibling's rounds. The
    // cells' replays run on threads of their own, joined in order.
    let cellgov: Vec<f64> = std::thread::scope(|scope| {
        let threads: Vec<_> = cells
            .iter()
            .map(|&(n, k)| scope.spawn(move || replay(n, k, iterations)))
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().expect("a cell's replay"))
            .collect()
    });
    for (&(n, k), &cellgov) in cells.iter().zip(&cellgov) {
        let console = console_median(&w, n, k);
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "{n} divides, {k} addi after each: CellGov {cellgov:.2} cycles an \
             iteration, the console {console:.2}"
        );
    }
}

#[test]
fn with_two_addi_after_each_divide_the_model_stalls_one_divide_early_where_the_console_does_not() {
    // CellGov differs here: one stage for each `addi` fills the model's
    // queue at 6 divides, and the console's at 7. The console's own step
    // for one divide more, with an `addi` after each, is a divide's stall.
    let w = console_words();
    let iterations = iterations(&w);
    let (five, six) = (console_median(&w, 5, 2), console_median(&w, 6, 2));
    assert!(
        (six - five).abs() / five < 0.02,
        "the console: 6 divides {six:.2}, 5 divides {five:.2}"
    );
    let stall = console_median(&w, 6, 1) - console_median(&w, 5, 1);
    assert!(
        replay(6, 2, iterations) - replay(5, 2, iterations) > stall / 2.0,
        "CellGov times 6 divides a stall of {stall:.2} slower than 5"
    );
    for &n in NS.iter().filter(|&&n| stalls_a_divide_early(n, 2)) {
        let console = console_median(&w, n, 2);
        let over = replay(n, 2, iterations) - console;
        assert!(
            over > stall / 2.0 && over < 1.5 * stall,
            "{n} divides: CellGov {over:.2} cycles over the console, a stall {stall:.2}"
        );
    }
}
