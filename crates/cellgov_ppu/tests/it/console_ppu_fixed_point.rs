//! CellGov's fixed-point instructions against the four fixed-point
//! console captures, case by case through the differential harness: the
//! arithmetic, the logic and compares, the rotates and shifts, and the
//! traps on conditions that do not hold.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

/// Each probe and its case count.
const PROBES: [(&str, usize); 4] = [
    ("ppu_fixed_point_arith", 1426),
    ("ppu_fixed_point_logic", 920),
    ("ppu_fixed_point_rotate", 1346),
    ("ppu_fixed_point_trap_quiet", 122),
];

#[test]
fn every_fixed_point_case_matches_the_console_on_both_runs() {
    for (probe, count) in PROBES {
        let cases = table(probe);
        let Capture::Done(runs) = capture(probe, cases.len()) else {
            panic!("{probe}: the run ended inside a case");
        };
        assert_eq!(runs.len(), count, "{probe}");
        let mut wrong = Vec::new();
        for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
            let v = varies(case, runs);
            if !v.is_empty() {
                wrong.push(format!("{}: the runs differ in {v:?}", case.id));
            }
            match replay(probe, case, index, &runs[0]) {
                CaseOutcome::Pass => {}
                other => wrong.push(format!("{} [{}]: {other:?}", case.id, case.asm)),
            }
        }
        assert!(
            wrong.is_empty(),
            "{probe}: {} case(s):\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }
}
