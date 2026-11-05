//! CellGov's PPU timing against the `ppu_issue_vscr_smt_runs_wide`
//! console capture: the `ppu_issue_vscr_smt_runs` chains beside siblings
//! of an `mtvscr` and 3 or 15 `vaddubm`.

use crate::console_ppu_issue_vscr_smt_runs::{
    cellgov_beside_sibling, median, shared_runs, words_at, CHAINS, META, PHASES, SIBLING_EXIT,
};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_vscr_smt_runs_wide/ps3/cech20-cex-493/observation.json";

/// The payload's word 566 when the probe wrote every word.
const DONE: u64 = 0x5050_5657;

/// The number of `vaddubm` after the `mtvscr` in the sibling of phases 1 and 2.
const SIBLINGS: [u32; 2] = [3, 15];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    words_at(CONSOLE, DONE)
}

/// The fewest runs of a chain the sibling must share for its median to
/// count.
const SHARED_FLOOR: usize = 5;

/// The instructions an iteration of each chain issues, its `bdnz` among
/// them.
const INSTRUCTIONS: [f64; CHAINS] = [2.0, 3.0, 4.0, 5.0, 7.0, 9.0, 13.0, 17.0, 11.0, 9.0];

#[test]
fn every_sibling_ran_and_shared_enough_runs_of_each_chain() {
    let w = console_words();
    for p in 1..PHASES {
        let m = META + 8 * p;
        assert_eq!(
            (w[m + 2], w[m + 3], w[m + 4], w[m + 5], w[m + 6]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {p}: the sibling's create rc, join rc, exit value, start and sleep rc",
        );
        for c in 0..CHAINS {
            let shared = shared_runs(&w, p, c).len();
            assert!(
                shared >= SHARED_FLOOR,
                "phase {p} chain {c}: {shared} shared runs"
            );
        }
    }
}

#[test]
fn beside_the_fifteen_vaddubm_sibling_each_instruction_costs_the_console_about_the_same() {
    let w = console_words();
    let mut per: Vec<f64> = (0..CHAINS)
        .map(|c| median(&shared_runs(&w, 2, c)) / INSTRUCTIONS[c])
        .collect();
    let rows = per.clone();
    per.sort_by(f64::total_cmp);
    let mid = median(&per);
    for (c, x) in rows.iter().enumerate() {
        assert!(
            (x - mid).abs() / mid < 0.2,
            "chain {c}: {x:.2} cycles an instruction, the median {mid:.2}: {rows:?}",
        );
    }
}

/// The share of the console's median shared run a held cell may lie on
/// either side of it, as in `console_ppu_issue_vscr_smt_runs`.
const BAND: f64 = 0.25;

/// The cells, as (phase, chain), the model times more than [`BAND`] under
/// the console's median shared run:
///
/// - beside the sibling of an `mtvscr` and 3 `vaddubm`: the chains of 2, 4
///   and 6 `vaddubm`, and of 8 `li`;
/// - beside the sibling of an `mtvscr` and 15 `vaddubm`: the chains of 2
///   and 4 `vaddubm`, of 2 `vaddubm` and 8 `li`, and of 8 `li`.
///
/// When a difference closes, the test fails. Then move the cell into the
/// held set.
const DIFFERENCES: [(usize, usize); 8] = [
    (1, 1),
    (1, 3),
    (1, 4),
    (1, 9),
    (2, 1),
    (2, 3),
    (2, 8),
    (2, 9),
];

/// Each cell beside a sibling, with CellGov's cycles and the console's
/// median shared run.
fn cells() -> Vec<(usize, usize, f64, f64)> {
    let w = console_words();
    (1..PHASES)
        .flat_map(|p| (0..CHAINS).map(move |c| (p, c)))
        .map(|(p, c)| {
            let cellgov = cellgov_beside_sibling(SIBLINGS[p - 1], c);
            (p, c, cellgov, median(&shared_runs(&w, p, c)))
        })
        .collect()
}

#[test]
fn each_held_chain_beside_a_wide_fenced_sibling_lies_within_the_band_of_the_median_shared_run() {
    let rows = cells();
    let held: Vec<_> = rows
        .iter()
        .filter(|(p, c, ..)| !DIFFERENCES.contains(&(*p, *c)))
        .collect();
    assert_eq!(held.len(), (PHASES - 1) * CHAINS - DIFFERENCES.len());
    for &&(p, c, cellgov, console) in &held {
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration, the console's \
             median {console:.2}, more than {BAND} of it apart: {rows:?}",
        );
    }
}

#[test]
fn each_other_chain_beside_a_wide_fenced_sibling_is_a_known_difference() {
    let rows = cells();
    for (p, c) in DIFFERENCES {
        let &(_, _, cellgov, console) = rows
            .iter()
            .find(|row| (row.0, row.1) == (p, c))
            .expect("the cell");
        assert!(
            cellgov < (1.0 - BAND) * console,
            "phase {p} chain {c}: CellGov {cellgov:.2} cycles an iteration now lies near the \
             console's median {console:.2}; move the cell into the held set",
        );
    }
}
