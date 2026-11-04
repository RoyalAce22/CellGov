//! CellGov's vaddfp and vmaddfp against the `ppu_vmx_float_policy`
//! console capture: rounding to nearest and the NJ underflow boundary.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vmx_float_policy";

#[test]
fn vaddfp_and_vmaddfp_match_the_console_in_both_nj_modes() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 36);
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
