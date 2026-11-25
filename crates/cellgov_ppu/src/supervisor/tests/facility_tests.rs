use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_VXU};

use super::exec_tests::{kernel_state, run, KERNEL_MSR};
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::state::PpuState;

/// `lfd f1,0(0)`, `stfd f1,0(0)`, `stfiwx f1,0,r2`, `fadd f1,f2,f3` and `mffs f1`.
const FLOATING_POINT: [u32; 5] = [
    0xC820_0000,
    0xD820_0000,
    0x7C20_17AE,
    0xFC22_182A,
    0xFC20_048E,
];

/// `vaddubm v1,v2,v3`, `vmaddfp v1,v2,v4,v3`, `mfvscr v1`, `lvsl v1,0,r2`,
/// `lvx v1,0,r2` and `stvx v1,0,r2`.
const VECTOR: [u32; 6] = [
    0x1022_1800,
    0x1022_192E,
    0x1020_0604,
    0x7C20_100C,
    0x7C20_10CE,
    0x7C20_11CE,
];

const FPR_SENTINEL: u64 = 0x1234_5678_9ABC_DEF0;
const VR_SENTINEL: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;

fn thread(msr: u64) -> PpuState {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = msr);
    state.set_fpr(1, FPR_SENTINEL);
    state.set_vr(1, VR_SENTINEL);
    state
}

fn fault(interrupt: PpuInterrupt) -> ExecuteVerdict {
    ExecuteVerdict::Fault(PpuFault::Interrupt(interrupt))
}

/// [PPC-Book3 p:72 s:5.5.10] A floating-point instruction, a load or store included, with MSR\[FP\] = 0 takes the floating-point unavailable interrupt.
#[test]
fn each_floating_point_form_with_fp_clear_takes_fp_unavailable_and_writes_nothing() {
    for raw in FLOATING_POINT {
        let mut state = thread(KERNEL_MSR | MSR_VXU);
        let before = state.state_hash();
        let verdict = run(&mut state, raw);
        assert_eq!(verdict, fault(PpuInterrupt::FpUnavailable), "0x{raw:08x}");
        assert_eq!(state.state_hash(), before, "0x{raw:08x}");
    }
}

/// [CBE-Handbook p:258 s:9.5.15] A VXU instruction with MSR\[VXU\] = 0 takes the VXU unavailable interrupt.
#[test]
fn each_vector_form_with_vxu_clear_takes_vxu_unavailable_and_writes_nothing() {
    for raw in VECTOR {
        let mut state = thread(KERNEL_MSR | MSR_FP);
        let before = state.state_hash();
        let verdict = run(&mut state, raw);
        assert_eq!(verdict, fault(PpuInterrupt::VxuUnavailable), "0x{raw:08x}");
        assert_eq!(state.state_hash(), before, "0x{raw:08x}");
    }
}

#[test]
fn a_floating_point_form_runs_with_fp_set_and_vxu_clear() {
    // fadd and mffs: neither reaches storage.
    for raw in [FLOATING_POINT[3], FLOATING_POINT[4]] {
        let mut state = thread(KERNEL_MSR | MSR_FP);
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Continue,
            "0x{raw:08x}"
        );
        assert_ne!(state.fpr[1], FPR_SENTINEL, "0x{raw:08x}");
    }
}

#[test]
fn a_vector_form_runs_with_vxu_set_and_fp_clear() {
    // vaddubm, vmaddfp and mfvscr: none reaches storage.
    for raw in [VECTOR[0], VECTOR[1], VECTOR[2]] {
        let mut state = thread(KERNEL_MSR | MSR_VXU);
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Continue,
            "0x{raw:08x}"
        );
        assert_ne!(state.vr[1], VR_SENTINEL, "0x{raw:08x}");
    }
}

/// [AltiVec-PEM p:2-9 s:2.5] The data stream instructions and the VRSAVE moves run with the vector unit disabled.
#[test]
fn the_data_stream_and_vrsave_moves_run_with_both_units_off() {
    // dst r1,r2,0, dstst r1,r2,0, dss 0, mfspr r1,256 and mtspr 256,r1.
    for raw in [
        0x7C01_12AC,
        0x7C01_12EC,
        0x7C00_066C,
        0x7C20_42A6,
        0x7C20_43A6,
    ] {
        let mut state = thread(KERNEL_MSR);
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Continue,
            "0x{raw:08x}"
        );
    }
}

#[test]
fn a_thread_with_no_supervisor_registers_runs_both_units() {
    for raw in [FLOATING_POINT[3], VECTOR[0]] {
        let mut state = PpuState::new();
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Continue,
            "0x{raw:08x}"
        );
    }
}
