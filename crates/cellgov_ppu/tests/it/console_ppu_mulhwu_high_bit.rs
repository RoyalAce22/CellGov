//! CellGov's `mulhwu` against the `ppu_mulhwu_high_bit` console capture: a
//! result with bit 32 set, in the plain and record forms.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBE: &str = "ppu_mulhwu_high_bit";

#[test]
fn mulhwu_with_bit_32_set_matches_the_console() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 2);
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
