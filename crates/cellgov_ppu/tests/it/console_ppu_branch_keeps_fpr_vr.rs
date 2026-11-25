//! CellGov's branches against the `ppu_branch_keeps_fpr_vr` console
//! capture: a taken b, bc, bdnz, bl, bcl, bctr, bctrl or blr keeps the
//! FPRs, the VRs and the FPSCR.

use crate::console_cases::{capture, run_with_branches, table, Capture, RUN_BASE};
use cellgov_ppu::differential::PpuStateSnapshot;
use cellgov_testkit::console_cases::Reg;

const PROBE: &str = "ppu_branch_keeps_fpr_vr";

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn each_taken_branch_form_keeps_the_fprs_the_vrs_and_the_fpscr_as_the_console_does() {
    let cases = table(PROBE);
    let Capture::Done(runs) = capture(PROBE, cases.len()) else {
        panic!("{PROBE}: the run ended inside a case");
    };
    assert_eq!(runs.len(), 9);
    for (index, (case, runs)) in cases.iter().zip(&runs).enumerate() {
        let state = run_with_branches(case, index);
        let at = format!("{} ({})", case.id, case.asm);
        assert_eq!(
            state.pc,
            RUN_BASE + 4 * case.words.len() as u64,
            "{at}: the run ends at the word after the last"
        );
        let cellgov = PpuStateSnapshot::capture(&state);
        for run in runs {
            for (reg, slot) in case.slots() {
                match reg {
                    Reg::Fpr(n) => {
                        assert_eq!(cellgov.fpr[usize::from(n)], run.slots[slot], "{at}: f{n}");
                    }
                    Reg::Vr(n) => {
                        let console =
                            u128::from(run.slots[slot]) << 64 | u128::from(run.slots[slot + 1]);
                        assert_eq!(cellgov.vr[usize::from(n)], console, "{at}: v{n}");
                    }
                    other => panic!("{at}: records {other:?}"),
                }
            }
            assert_eq!(cellgov.cr, run.cr, "{at}: CR");
            assert_eq!(cellgov.xer, run.xer, "{at}: XER");
            assert_eq!(cellgov.fpscr, run.fpscr, "{at}: FPSCR");
        }
    }
}
