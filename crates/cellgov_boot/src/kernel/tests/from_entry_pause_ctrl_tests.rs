use super::*;

/// `b` to the word 4 words back.
const B_BACK_4: u32 = 0x4BFF_FFF0;

#[test]
fn thread_1_reads_only_its_own_te_bit_while_lv1_holds_thread_0_paused() {
    let alone = CTRL_CT_THREAD1 | CTRL_TE_THREAD1;
    let code = [
        MFCTRL_R3,
        ANDIS_R3_CT1,
        bne(4),
        LI_R3_0,
        LI_R11_PAUSE,
        SC_1, // 5: thread 0 pauses
        MFCTRL_R3,
        lis(4, alone >> 16),
        CMPLW_R3_R4,
        beq(2),
        B_BACK_4,
        B_SELF, // 11: thread 1 read only its own thread in TE
    ];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!(thread0.status, Some(UnitStatus::Blocked));
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(5), thread1_pc(11)));
}

#[test]
fn thread_0_reads_only_its_own_te_bit_while_lv1_holds_thread_1_paused() {
    let alone = CTRL_CT_THREAD0 | CTRL_TE_THREAD0;
    let code = [
        MFCTRL_R3,
        ANDIS_R3_CT1,
        beq(4),
        LI_R3_0,
        LI_R11_PAUSE,
        SC_1, // 5: thread 1 pauses
        MFCTRL_R3,
        lis(4, alone >> 16),
        CMPLW_R3_R4,
        beq(2),
        B_BACK_4,
        B_SELF, // 11: thread 0 read only its own thread in TE
    ];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!(thread1.status, Some(UnitStatus::Blocked));
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(11), thread1_pc(5)));
}
