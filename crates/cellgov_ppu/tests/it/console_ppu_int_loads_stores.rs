//! CellGov's integer loads and stores against their console captures, at
//! aligned and misaligned addresses, case by case through the
//! differential harness.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBES: [(&str, usize); 3] = [
    ("ppu_int_loads", 194),
    ("ppu_int_stores", 280),
    ("ppu_int_misaligned", 208),
];

#[test]
fn every_integer_load_and_store_case_matches_the_console_on_both_runs() {
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
