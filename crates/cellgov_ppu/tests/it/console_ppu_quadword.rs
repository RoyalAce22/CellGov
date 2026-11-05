//! CellGov's `lq` and `stq` against their may-trap console captures: each
//! ends the process on the console, and CellGov refuses each with its
//! class.

use crate::console_cases::{capture, cellgov_after, table, Capture};
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::exec::{ExecuteVerdict, PpuFault};

#[test]
fn lq_and_stq_end_the_process_on_the_console_and_refuse_in_cellgov() {
    for (name, fault) in [
        ("ppu_may_trap_lq", PpuFault::privileged("lq")),
        ("ppu_may_trap_lq_rt_ra", PpuFault::illegal("lq")),
        ("ppu_may_trap_stq", PpuFault::privileged("stq")),
    ] {
        let cases = table(name);
        assert_eq!(cases.len(), 1, "{name}: one case");
        assert!(cases[0].may_end_process, "{name}: a may-trap case");
        assert!(
            matches!(capture(name, 1), Capture::Ended { case: 0 }),
            "{name}: the console ran it"
        );
        assert_eq!(
            cellgov_after(&cases[0], 0).map(|_| ()).map_err(|e| *e),
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(fault))),
            "{name}"
        );
    }
}
