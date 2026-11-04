//! CellGov's `twi` and `tdi` against the console: a trap whose
//! conditions do not hold runs on, and one whose condition holds ends
//! the process.

use crate::console_cases::{capture, cellgov_after, replay, table, varies, Capture};
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::exec::{ExecuteVerdict, PpuFault};

#[test]
fn a_trap_immediate_whose_conditions_do_not_hold_runs_on_on_the_console_and_in_cellgov() {
    let probe = "ppu_trap_immediate";
    let cases = table(probe);
    let Capture::Done(runs) = capture(probe, cases.len()) else {
        panic!("{probe}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 5);
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        assert!(
            varies(case, runs).is_empty(),
            "{}: the runs differ",
            case.id
        );
        assert_eq!(
            replay(probe, case, index, &runs[0]),
            CaseOutcome::Pass,
            "{} [{}]",
            case.id,
            case.asm
        );
    }
}

#[test]
fn a_trap_immediate_whose_condition_holds_ends_the_process_on_the_console_and_traps_in_cellgov() {
    // twi 4 fires on RA's low word alone; tdi 2 compares against EXTS(-1).
    for (probe, to) in [("ppu_may_trap_twi_low", 4), ("ppu_may_trap_tdi_ltu", 2)] {
        let cases = table(probe);
        assert_eq!(cases.len(), 1, "{probe}: one case");
        assert!(cases[0].may_end_process, "{probe}: a may-trap case");
        let console = capture(probe, 1);
        assert!(
            matches!(console, Capture::Ended { case: 0 }),
            "{probe}: the console ran it: {console:?}"
        );
        let cellgov = cellgov_after(&cases[0], 0).map(|_| ()).map_err(|e| *e);
        assert_eq!(
            cellgov,
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                PpuFault::ProgramTrap(to)
            ))),
            "{probe}"
        );
    }
}
