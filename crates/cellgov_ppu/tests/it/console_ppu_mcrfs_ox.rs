//! CellGov's `mcrfs` against the `ppu_mcrfs_ox` console capture: what it
//! copies from FPSCR field 0 when OX is set, however OX was set.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_mcrfs_ox";

#[test]
fn mcrfs_of_field_0_matches_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 18);
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
