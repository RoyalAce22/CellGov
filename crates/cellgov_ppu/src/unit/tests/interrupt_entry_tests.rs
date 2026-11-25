//! The MSR and SRR1 that interrupt entry gives.

use super::{entry_msr, entry_srr1};
use cellgov_ps3_abi::hw::ppu_interrupt::{SRR1_INTERRUPT_INFO, SRR1_PROGRAM_TRAP};
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_BE, MSR_DR, MSR_EE, MSR_FE0, MSR_FE1, MSR_FP, MSR_HV, MSR_IR, MSR_ME, MSR_PMM, MSR_PR,
    MSR_RI, MSR_SE, MSR_SF,
};

/// [CBE-Handbook p:292 s:9.12] Table 9-40 note 1: BE, EE, FE0, FE1, FP, PR, SE, DR and IR clear and SF sets; RI clears; ME stays; HV stays under LPCR\[LPES\] = 0b11.
#[test]
fn interrupt_entry_clears_the_table_9_40_bits_sets_sf_and_keeps_me_and_hv() {
    let cleared = [
        MSR_BE, MSR_EE, MSR_FE0, MSR_FE1, MSR_FP, MSR_PR, MSR_SE, MSR_DR, MSR_IR, MSR_RI,
    ];
    let every = cleared.iter().fold(0, |m, b| m | b);
    assert_eq!(entry_msr(every), MSR_SF);
    for bit in cleared {
        assert_eq!(entry_msr(bit), MSR_SF, "{bit:#x}");
    }
    assert_eq!(entry_msr(MSR_ME), MSR_SF | MSR_ME);
    assert_eq!(entry_msr(MSR_HV), MSR_SF | MSR_HV);
    assert_eq!(
        entry_msr(MSR_SF | MSR_HV | MSR_ME),
        MSR_SF | MSR_HV | MSR_ME
    );
    assert_eq!(entry_msr(0), MSR_SF, "a 32-bit MSR enters in 64-bit mode");
}

/// [PPC-Book3 p:65 s:5.5] Figure 35: an interrupt sets PMM to 0.
#[test]
fn interrupt_entry_clears_the_performance_monitor_mark() {
    assert_eq!(entry_msr(MSR_PMM), MSR_SF);
    assert_eq!(entry_msr(MSR_SF | MSR_ME | MSR_PMM), MSR_SF | MSR_ME);
}

/// [PPC-Book3 p:62 s:5.4] SRR1 bits 33:36 and 42:47 take the interrupt's information, and the rest the MSR.
#[test]
fn srr1_copies_the_msr_outside_the_interrupt_bits_and_the_cause_inside_them() {
    let msr = !0u64;
    assert_eq!(entry_srr1(msr, 0), !SRR1_INTERRUPT_INFO);
    assert_eq!(
        entry_srr1(msr, SRR1_PROGRAM_TRAP),
        !SRR1_INTERRUPT_INFO | SRR1_PROGRAM_TRAP
    );
    let user = MSR_SF | MSR_EE | MSR_PR | MSR_IR | MSR_DR | MSR_ME;
    assert_eq!(
        entry_srr1(user, SRR1_PROGRAM_TRAP),
        user | SRR1_PROGRAM_TRAP
    );
    assert_eq!(
        entry_srr1(0, !0),
        SRR1_INTERRUPT_INFO,
        "a cause outside bits 33:36 and 42:47 does not reach SRR1"
    );
}
