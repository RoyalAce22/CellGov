//! CellGov's integer vector multiply-add and multiply-sum instructions
//! against the `ppu_vmul_add_sum` console capture: vD and the VSCR each
//! leaves.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vmul_add_sum";

#[test]
fn the_multiply_adds_and_sums_and_their_sat_match_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 16);
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        assert!(
            varies(case, runs).is_empty(),
            "{}: the runs differ",
            case.id
        );
        assert_eq!(
            replay(PROBE, case, index, &runs[0]),
            CaseOutcome::Pass,
            "{} [{}]",
            case.id,
            case.asm
        );
    }
}
