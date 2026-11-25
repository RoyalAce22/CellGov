use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_SE};

use super::exec_tests::{kernel_state, run, KERNEL_MSR};
use crate::exec::{ExecuteVerdict, PpuFault};

/// `mtmsr r5`.
const MTMSR_R5: u32 = 0x7CA0_0124;

#[test]
fn an_mtmsr_that_changes_se_refuses_and_leaves_the_msr() {
    for (msr, rs) in [(KERNEL_MSR, MSR_SE), (KERNEL_MSR | MSR_SE, 0)] {
        let mut state = kernel_state();
        state.update_supervisor(|s| s.msr = msr);
        state.set_gpr(5, rs);
        assert_eq!(
            run(&mut state, MTMSR_R5),
            ExecuteVerdict::Fault(PpuFault::Unmodelled("an mtmsr that changes MSR[SE]"))
        );
        assert_eq!(state.supervisor().map(|s| s.msr), Some(msr));
    }
}

/// [PPC-Book3 p:91 s:8.3] mtmsr with L = 1 writes only EE and RI, so RS bit 53 cannot change SE.
#[test]
fn an_mtmsr_with_l_1_runs_whatever_rs_holds_for_se() {
    // mtmsr r5,1.
    const MTMSR_R5_L1: u32 = MTMSR_R5 | (1 << 16);
    for (msr, rs) in [(KERNEL_MSR, MSR_SE | MSR_EE), (KERNEL_MSR | MSR_SE, MSR_EE)] {
        let mut state = kernel_state();
        state.update_supervisor(|s| s.msr = msr);
        state.set_gpr(5, rs);
        assert_eq!(run(&mut state, MTMSR_R5_L1), ExecuteVerdict::Continue);
        assert_eq!(state.supervisor().map(|s| s.msr), Some(msr | MSR_EE));
    }
}

#[test]
fn an_mtmsr_that_keeps_se_runs() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = KERNEL_MSR | MSR_SE);
    state.set_gpr(5, MSR_SE | MSR_EE);
    assert_eq!(run(&mut state, MTMSR_R5), ExecuteVerdict::Continue);
    assert_eq!(
        state.supervisor().map(|s| s.msr),
        Some(KERNEL_MSR | MSR_SE | MSR_EE)
    );
}
