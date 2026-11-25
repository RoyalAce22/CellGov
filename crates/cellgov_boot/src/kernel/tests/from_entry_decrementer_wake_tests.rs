use super::*;

/// `mtspr 22, r4`: `mtdec r4`.
const MTDEC_R4: u32 = 0x7C96_03A6;

/// `li r4, 64`: a decrementer that reaches 0 after 64 time-base ticks.
const LI_R4_64: u32 = 0x3880_0040;

/// `lis r4, 0x7fff`: a decrementer that reaches 0 after about 2^31
/// time-base ticks, past any step cap of these tests.
const LIS_R4_7FFF: u32 = 0x3C80_7FFF;

/// `or r8, r3, r4`.
const OR_R8_R3_R4: u32 = 0x7C68_2378;

/// Sets the decrementer from `load`, pauses in mode 0, and spins at word 5.
fn pause_after(load: u32) -> [u32; 6] {
    [load, MTDEC_R4, LI_R3_0, LI_R11_PAUSE, SC_1, B_SELF]
}

#[test]
fn a_mode_0_pause_of_both_threads_ends_at_the_decrementer_and_both_threads_run_on() {
    let elf = shared_entry_elf(&pause_after(LI_R4_64));
    let run = run_kernel_from_entry(&elf, FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(5), thread1_pc(5)));
    assert_eq!(
        (thread0.status, thread1.status),
        (Some(UnitStatus::Runnable), Some(UnitStatus::Runnable))
    );
    assert_eq!(
        (
            thread0.last_hypercall.map(|c| c.number),
            thread1.last_hypercall.map(|c| c.number)
        ),
        (Some(PAUSE), Some(PAUSE))
    );
}

#[test]
fn a_mode_0_pause_ends_when_its_decrementer_reaches_0_while_the_other_thread_runs() {
    let mut code = pause_after(LI_R4_64).to_vec();
    code.pop();
    code.extend([OR_R8_R3_R4]);
    code.extend(send_r8());
    code.push(B_SELF);
    let spin = u64::try_from(code.len()).expect("a short program") - 1;
    let run = run_kernel_from_entry(&kernel_elf(&code), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    assert_eq!(thread0.status, Some(UnitStatus::Runnable));
    assert_eq!(thread0.pc, ENTRY + 4 * spin);
    // lv1's handler returns r3 = 0 whatever ended the pause, and its return
    // clears r4, which held the decrementer. The program sends r3 or r4.
    assert_eq!(run.console, 0u64.to_be_bytes().to_vec());
}

#[test]
fn a_mode_1_pause_holds_its_thread_while_the_other_thread_s_mode_0_pause_ends_at_the_decrementer() {
    // Thread 0 pauses in mode 1 and thread 1 in mode 0.
    let code = [
        LI_R4_64,
        MTDEC_R4,
        MFCTRL_R3,
        ANDIS_R3_CT1,
        LI_R3_1,
        beq(2),
        LI_R3_0,
        LI_R11_PAUSE,
        SC_1,
        B_SELF,
    ];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(8), thread1_pc(9)));
    assert_eq!(
        (thread0.status, thread1.status),
        (Some(UnitStatus::Blocked), Some(UnitStatus::Runnable))
    );
}

#[test]
fn a_mode_0_pause_holds_its_thread_until_its_decrementer_reaches_0() {
    let run = run_kernel_from_entry(&kernel_elf(&pause_after(LIS_R4_7FFF)), FW_PAUSE, 200).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!(thread0.status, Some(UnitStatus::Blocked));
    assert_eq!(thread0.pc, ENTRY + 16);
    assert_eq!(thread1.status, Some(UnitStatus::Runnable));
}
