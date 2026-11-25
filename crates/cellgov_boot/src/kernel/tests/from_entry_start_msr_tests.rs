use super::*;

/// The start MSR that lv1 gives the logical PPE of a partition: SF only.
///
/// - The logical PPE constructor builds the value and stores it at +176
///   [OFW-4.93 lv1 a:0x2E0E60-0x2E0EA0].
/// - The first entry loads +176 [OFW-4.93 lv1 a:0x2E1938] and moves it
///   to SRR1 before `rfid` [OFW-4.93 lv1 a:0x2C427C-0x2C4280].
const LV1_START_MSR: u64 = 0x8000_0000_0000_0000;

#[test]
fn thread_0_enters_the_kernel_with_the_start_msr_that_lv1_gives_the_logical_ppe() {
    let run = run_kernel_from_entry(&kernel_elf(&[B_SELF]), FW, 8).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let kernel = run.thread0.expect("the run gives thread 0's position");
    assert_eq!(kernel.pc, ENTRY);
    assert_eq!(kernel.msr, LV1_START_MSR);
}
