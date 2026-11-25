use cellgov_exec::UnitStatus;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_SF};
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;

use super::super::{KernelHypercall, RECENT_PCS};
use super::*;

const NOP: u32 = 0x6000_0000;
const MFMSR_R8: u32 = 0x7C00_00A6 | (8 << 21);
/// `ori r9, r8, MSR_DR`.
const ORI_R9_R8_DR: u32 = 0x6000_0000 | (8 << 21) | (9 << 16) | MSR_DR as u32;
const MTMSRD_R9: u32 = 0x7C00_0164 | (9 << 21);
const ISYNC: u32 = 0x4C00_012C;

#[test]
fn a_capped_run_names_the_kernels_last_hypercall_and_the_sc_that_made_it() {
    let code = &[
        LI_R3_2, LIS_R4_BI, LIS_R5_PU, ORI_R5_N, SLDI_R5_32, LI_R11_91, SC_1, B_SELF,
    ];
    let run = run_kernel_from_entry(&kernel_elf(code), FW, 100).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let kernel = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(
        kernel.last_hypercall,
        Some(KernelHypercall {
            number: READ_REPOSITORY_NODE,
            pc: ENTRY + 4 * 6,
            step: 1,
        })
    );
    assert_eq!(kernel.pc, ENTRY + 4 * 7);
    assert_eq!(kernel.recent_pcs, vec![ENTRY + 4 * 7; RECENT_PCS]);
}

#[test]
fn a_capped_run_of_a_kernel_that_makes_no_hypercall_names_its_spin_pc_and_no_call() {
    let run = run_kernel_from_entry(&kernel_elf(&[B_SELF]), FW, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let kernel = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(kernel.pc, ENTRY);
    assert_eq!(kernel.msr, MSR_SF);
    assert_eq!(kernel.status, Some(UnitStatus::Runnable));
    assert_eq!(kernel.last_hypercall, None);
}

#[test]
fn the_recent_pcs_keep_the_newest_step_ends_oldest_first() {
    // 4096 straight-line words outlast the run, so each kernel step ends
    // at a higher pc than the step before it. The two units take turns,
    // so a cap two steps lower ends the run one kernel step earlier.
    let mut code = vec![NOP; 4096];
    code.push(B_SELF);
    let elf = kernel_elf(&code);
    let run = run_kernel_from_entry(&elf, FW, 30).unwrap();
    let shorter = run_kernel_from_entry(&elf, FW, 28).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let kernel = run.thread0.expect("the run gives thread 0's position");
    let earlier = shorter.thread0.expect("the run gives thread 0's position");
    assert_eq!(kernel.recent_pcs.len(), RECENT_PCS);
    assert!(
        kernel.recent_pcs.windows(2).all(|pair| pair[0] < pair[1]),
        "{:x?}",
        kernel.recent_pcs
    );
    assert_eq!(
        kernel.recent_pcs[..RECENT_PCS - 1],
        earlier.recent_pcs[1..],
        "a step more drops the oldest pc and keeps the rest"
    );
    assert_eq!(kernel.recent_pcs.last(), Some(&kernel.pc));
    assert!(kernel.pc < ENTRY + 4 * 4096, "0x{:x}", kernel.pc);
}

#[test]
fn a_refused_run_leaves_the_kernels_pc_on_the_refused_sc_and_names_that_call() {
    let run = run_kernel_from_entry(&kernel_elf(TWO_READS), FW, 10_000).unwrap();
    assert!(
        matches!(run.stop, KernelRunStop::Refused(_)),
        "{:?}",
        run.stop
    );
    let kernel = run.thread0.expect("the run gives thread 0's position");
    let second_sc = ENTRY + 4 * 11;
    assert_eq!(kernel.pc, second_sc);
    let call = kernel
        .last_hypercall
        .expect("the refused call is the last hypercall");
    assert_eq!((call.number, call.pc), (READ_REPOSITORY_NODE, second_sc));
    assert_eq!(kernel.recent_pcs.last(), Some(&second_sc));
}

#[test]
fn a_kernel_step_that_faults_adds_no_recent_pc_and_leaves_the_pc_where_it_began() {
    let run = run_kernel_from_entry(&kernel_elf(&[MFSPR_R0_PVR]), FW, 10_000).unwrap();
    assert!(
        matches!(run.stop, KernelRunStop::Fault { pc: Some(pc), .. } if pc == ENTRY),
        "{:?}",
        run.stop
    );
    let kernel = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(kernel.pc, ENTRY);
    assert!(kernel.recent_pcs.is_empty(), "{:x?}", kernel.recent_pcs);
}

#[test]
fn the_position_gives_the_msr_that_the_kernel_set() {
    let code = &[MFMSR_R8, ORI_R9_R8_DR, MTMSRD_R9, ISYNC, B_SELF];
    let run = run_kernel_from_entry(&kernel_elf(code), FW, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let kernel = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(kernel.msr, MSR_SF | MSR_DR);
    assert_eq!(kernel.pc, ENTRY + 4 * 4);
}
