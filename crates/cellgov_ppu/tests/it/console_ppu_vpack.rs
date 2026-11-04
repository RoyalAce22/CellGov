//! CellGov's vector pack instructions against the `ppu_vpack` console
//! capture: vD and the VSCR each leaves.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vpack";

#[test]
fn the_pack_ops_and_their_sat_match_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 21);
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
