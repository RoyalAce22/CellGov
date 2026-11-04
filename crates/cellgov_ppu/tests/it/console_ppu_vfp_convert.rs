//! CellGov's vcfsx, vcfux, vctsxs and vctuxs against the
//! `ppu_vfp_convert` console capture: ties, word edges, saturation and
//! SAT, NaNs, infinities and denormals in both NJ modes.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vfp_convert";

#[test]
fn the_conversions_match_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 46);
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
