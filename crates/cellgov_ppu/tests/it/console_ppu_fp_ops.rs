//! CellGov's scalar floating point against seven console captures.
//!
//! The `ppu_fp_arith`, `ppu_fp_fma` and `ppu_fp_unary` captures run each
//! instruction:
//!
//! - on each operand class;
//! - in each rounding mode;
//! - with each exception raised, its enable bit clear and set;
//! - in its record form.
//!
//! The `ppu_fp_loads` and `ppu_fp_stores` captures move a datum of each
//! class through each address form. The `ppu_fpscr_ops` and
//! `ppu_fpscr_rules` captures run the FPSCR moves and the FPSCR's own
//! rules.

use crate::console_cases::{capture, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;

const PROBES: [(&str, usize); 7] = [
    ("ppu_fp_arith", 266),
    ("ppu_fp_fma", 360),
    ("ppu_fp_unary", 334),
    ("ppu_fp_loads", 98),
    ("ppu_fp_stores", 131),
    ("ppu_fpscr_ops", 147),
    ("ppu_fpscr_rules", 12),
];

#[test]
fn every_scalar_floating_point_case_matches_the_console_on_both_runs() {
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
