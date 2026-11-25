//! BP_VR, the CBEA-compliant processor version register, in each state.

use cellgov_ps3_abi::hw::ppu::CELL_BP_VR;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_EE, MSR_HV, MSR_IR, MSR_PR, SPR_BP_VR};

use super::exec_tests::{kernel_state, mfspr, mtspr, run, KERNEL_MSR};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;

/// [PPC-Book3 p:21 s:3.4.2] mfspr of a 32-bit SPR places it in RT bits 32:63 and clears bits 0:31.
#[test]
fn mfspr_of_bp_vr_in_supervisor_state_reads_the_modelled_value_into_the_low_word() {
    for msr in [KERNEL_MSR, KERNEL_MSR | MSR_HV] {
        let mut state = kernel_state();
        state.update_supervisor(|s| s.msr = msr);
        state.set_gpr(3, !0);
        assert_eq!(
            run(&mut state, mfspr(3, SPR_BP_VR)),
            ExecuteVerdict::Continue
        );
        assert_eq!(state.gpr[3], u64::from(CELL_BP_VR), "msr {msr:#x}");
    }
}

/// [CBEA p:281 s:23.1] BP_VR provides no write access.
/// [CBEA p:312 s:Appendix E] E.4: in privileged state, an access to an inaccessible SPR executes as a no-op.
#[test]
fn mtspr_of_bp_vr_in_supervisor_state_changes_nothing() {
    for msr in [KERNEL_MSR, KERNEL_MSR | MSR_HV] {
        let mut state = kernel_state();
        state.update_supervisor(|s| s.msr = msr);
        state.set_gpr(5, 0xFFFF_FFFF_DEAD_BEEF);
        let before = PpuArchitecturalState::capture(&state);
        let hash = state.state_hash();
        assert_eq!(
            run(&mut state, mtspr(SPR_BP_VR, 5)),
            ExecuteVerdict::Continue
        );
        assert_eq!(
            PpuArchitecturalState::capture(&state),
            before,
            "msr {msr:#x}"
        );
        assert_eq!(state.state_hash(), hash, "msr {msr:#x}");
        run(&mut state, mfspr(6, SPR_BP_VR));
        assert_eq!(state.gpr[6], u64::from(CELL_BP_VR), "msr {msr:#x}");
    }
}

/// [CBE-Handbook p:254 s:9.5.9] The PPE takes a privileged-instruction interrupt for a problem-state access to a supervisor SPR.
/// [PPC-Book3 p:21 s:3.4.2] spr0 = 1 marks the read as privileged; SPR 1022 has spr0 = 1.
///
/// The reference console ends a process at a problem-state `mfspr` of
/// SPR 1022 (probe `ppu_may_trap_mfspr1022`).
#[test]
fn both_bp_vr_moves_refuse_as_privileged_in_problem_state_and_change_nothing() {
    let mut kernel = kernel_state();
    kernel.update_supervisor(|s| s.msr = KERNEL_MSR | MSR_PR | MSR_EE | MSR_IR | MSR_DR);
    for mut state in [kernel, PpuState::new()] {
        state.set_gpr(3, 0x1234);
        let before = PpuArchitecturalState::capture(&state);
        let hash = state.state_hash();
        for (raw, mnemonic) in [
            (mfspr(3, SPR_BP_VR), "mfspr"),
            (mtspr(SPR_BP_VR, 3), "mtspr"),
        ] {
            assert_eq!(
                run(&mut state, raw),
                ExecuteVerdict::Fault(PpuFault::privileged(mnemonic))
            );
            assert_eq!(PpuArchitecturalState::capture(&state), before, "{mnemonic}");
            assert_eq!(state.state_hash(), hash, "{mnemonic}");
        }
    }
}
