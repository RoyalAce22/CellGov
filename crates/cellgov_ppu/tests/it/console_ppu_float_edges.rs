//! CellGov's scalar floating point against the `ppu_float_edges` console
//! capture: each arithmetic, multiply-add, divide, square-root and
//! conversion op in every rounding mode, each FPRF class and VX cause,
//! sticky exception bits, the record forms' CR1, `mcrfs` and `stfs`.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_float_edges";

#[test]
fn every_float_edge_matches_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 120);
    let mut failed = Vec::new();
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        assert!(
            varies(case, runs).is_empty(),
            "{}: the runs differ",
            case.id
        );
        let outcome = replay(PROBE, case, index, &runs[0]);
        if outcome != CaseOutcome::Pass {
            failed.push(format!("{} [{}]: {outcome:?}", case.id, case.asm));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
