//! [CBE-Handbook p:739 s:A.2.5] The PPE supports only big-endian mode, and a write to MSR\[LE\] or MSR\[ILE\] has no effect.

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_ILE, MSR_LE, MSR_SF};

use super::exec_tests::{kernel_state, run, KERNEL_MSR};
use super::SupervisorState;
use crate::exec::ExecuteVerdict;
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;

/// `mtmsrd r3`.
const MTMSRD_R3: u32 = 0x7C60_0164;
/// `mtmsr r3`.
const MTMSR_R3: u32 = 0x7C60_0124;
/// `mfmsr r4`.
const MFMSR_R4: u32 = 0x7C80_00A6;
/// `rfid`.
const RFID: u32 = 0x4C00_0024;

fn msr(state: &PpuState) -> u64 {
    state.supervisor().expect("supervisor registers").msr
}

#[test]
fn mtmsrd_and_mtmsr_leave_ile_and_le_at_0_and_mfmsr_reads_them_0() {
    for raw in [MTMSRD_R3, MTMSR_R3] {
        let mut state = kernel_state();
        state.set_gpr(3, KERNEL_MSR | MSR_EE | MSR_ILE | MSR_LE);
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Continue,
            "0x{raw:08x}"
        );
        assert_eq!(run(&mut state, MFMSR_R4), ExecuteVerdict::Continue);
        assert_eq!(state.gpr[4], KERNEL_MSR | MSR_EE, "0x{raw:08x}");
    }
}

#[test]
fn rfid_leaves_le_at_0_whatever_srr1_holds() {
    let mut state = kernel_state();
    state.update_supervisor(|s| {
        s.srr0 = 0x100;
        s.srr1 = MSR_SF | MSR_EE | MSR_ILE | MSR_LE;
    });
    assert_eq!(run(&mut state, RFID), ExecuteVerdict::Branch);
    assert_eq!(state.pc, 0x100);
    assert_eq!(msr(&state), MSR_SF | MSR_EE);
}

#[test]
fn installed_supervisor_registers_hold_ile_and_le_at_0() {
    let mut big = PpuState::new();
    big.set_supervisor(Some(SupervisorState::new(MSR_SF)));
    let mut asked_little = PpuState::new();
    asked_little.set_supervisor(Some(SupervisorState::new(MSR_SF | MSR_ILE | MSR_LE)));
    assert_eq!(msr(&asked_little), MSR_SF);
    assert!(asked_little.hash_is_current());
    assert_eq!(asked_little.state_hash(), big.state_hash());
}

#[test]
fn an_applied_observation_holds_ile_and_le_at_0() {
    let mut observed = PpuArchitecturalState::capture(&kernel_state());
    observed
        .supervisor
        .as_mut()
        .expect("captured supervisor registers")
        .msr |= MSR_ILE | MSR_LE;
    let mut state = PpuState::new();
    observed.apply(&mut state);
    assert_eq!(msr(&state), KERNEL_MSR);
    assert!(state.hash_is_current());
}
