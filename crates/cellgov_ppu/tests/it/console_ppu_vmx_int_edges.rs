//! CellGov's vector integer families on one edge table against the
//! `ppu_vmx_int_edges` console capture: vD, the VSCR each saturating
//! case leaves, and CR after each recording compare.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vmx_int_edges";

#[test]
fn every_integer_family_matches_the_console_on_the_edge_table() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 86);
    let mismatches: Vec<String> = cases
        .iter()
        .zip(&runs)
        .enumerate()
        .filter_map(|(index, (case, runs))| {
            let differ = !varies(case, runs).is_empty();
            let outcome = replay(PROBE, case, index, &runs[0]);
            (differ || outcome != CaseOutcome::Pass)
                .then(|| format!("{} [{}]: varies {differ}, {outcome:?}", case.id, case.asm))
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
