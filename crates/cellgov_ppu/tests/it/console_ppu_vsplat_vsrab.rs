//! CellGov's vsrab count width, out-of-range splat index and vperm index
//! bits against the `ppu_vsplat_vsrab` console capture.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_vsplat_vsrab";

#[test]
fn the_open_splat_and_shift_results_match_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 9);
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
