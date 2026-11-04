//! CellGov's zero-count `lswx` against the `ppu_may_trap_lswx_n0_null`
//! console capture: the console reads the byte at EA even with a zero
//! count, so an unmapped EA ends the process.

use crate::console_cases::{capture, cellgov_after, table, Capture};
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::exec::ExecuteVerdict;

const PROBE: &str = "ppu_may_trap_lswx_n0_null";

#[test]
fn a_zero_count_lswx_at_an_unmapped_address_ends_the_process_and_faults_in_cellgov() {
    let cases = table(PROBE);
    assert_eq!(cases.len(), 1, "{PROBE}: one case");
    let console = capture(PROBE, 1);
    assert!(
        matches!(console, Capture::Ended { case: 0 }),
        "{PROBE}: the console ran it: {console:?}"
    );
    let cellgov = cellgov_after(&cases[0], 0).map(|_| ()).map_err(|e| *e);
    assert!(
        matches!(
            cellgov,
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::MemFault(_)))
        ),
        "{PROBE}: CellGov: {cellgov:?}"
    );
}
