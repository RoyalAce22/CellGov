use std::collections::BTreeSet;

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_RESET_THREAD1;
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;

use super::super::KernelHypercallSite;
use super::*;

/// `b` to the word 7 words back: the first word of [`READ_LOOP`].
const B_BACK_7: u32 = 0x4BFF_FFE4;

/// A read of `bi.pun` from one `sc`, again and again.
const READ_LOOP: &[u32] = &[
    LI_R3_2, LIS_R4_BI, LIS_R5_PU, ORI_R5_N, SLDI_R5_32, LI_R11_91, SC_1, B_BACK_7,
];

/// The index of the `sc` in [`READ_LOOP`].
const READ_LOOP_SC: u64 = 6;

fn site(pc: u64, number: u64) -> KernelHypercallSite {
    KernelHypercallSite { pc, number }
}

#[test]
fn a_run_keeps_each_hypercall_site_of_thread_0_with_the_refused_one_and_none_for_thread_1() {
    let run = run_kernel_from_entry(&kernel_elf(TWO_READS), FW, 10_000).unwrap();
    assert!(
        matches!(run.stop, KernelRunStop::Refused(_)),
        "{:?}",
        run.stop
    );
    let expected = BTreeSet::from([
        site(ENTRY + 4 * 6, READ_REPOSITORY_NODE),
        site(ENTRY + 4 * 11, READ_REPOSITORY_NODE),
    ]);
    assert_eq!(run.hypercall_sites, [expected, BTreeSet::new()]);
}

#[test]
fn a_hypercall_that_a_thread_repeats_at_one_sc_is_one_site_of_that_thread() {
    // Both threads start at the first word: thread 0 at the link address,
    // thread 1 at its reset vector with no high address bit.
    let run = run_kernel_from_entry(&elf_image(BASE, 0, READ_LOOP, 0x100), FW, 400).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    for unit in [UnitId::new(0), UnitId::new(1)] {
        let reads = run
            .repository_reads
            .iter()
            .filter(|r| r.unit == unit)
            .count();
        assert!(reads >= 2, "unit {unit:?} made {reads} reads");
    }
    let sc = 4 * READ_LOOP_SC;
    assert_eq!(
        run.hypercall_sites,
        [
            BTreeSet::from([site(BASE + 0x100 + sc, READ_REPOSITORY_NODE)]),
            BTreeSet::from([site(VECTOR_SYSTEM_RESET_THREAD1 + sc, READ_REPOSITORY_NODE)]),
        ]
    );
}
