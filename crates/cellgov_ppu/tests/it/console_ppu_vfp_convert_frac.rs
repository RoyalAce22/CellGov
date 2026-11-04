//! CellGov's vctsxs and vctuxs against the `ppu_vfp_convert_frac`
//! console capture: negative products that truncate to zero, where SAT
//! answers whether vctuxs saturates a product in (-1, 0).

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vfp_convert_frac";

#[test]
fn a_product_in_minus_one_to_zero_matches_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 20);
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
