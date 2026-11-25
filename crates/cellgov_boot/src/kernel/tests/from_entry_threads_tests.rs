use cellgov_exec::UnitStatus;
use cellgov_lv1::PausedThread;
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_SYSTEM_RESET_THREAD1;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_R};
use cellgov_ps3_abi::hw::ppu_spr::{
    CTRL_CT_THREAD0, CTRL_CT_THREAD1, CTRL_TE_THREAD0, CTRL_TE_THREAD1,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_SF, SLB_VALID};
use cellgov_ps3_abi::lv1::call::{
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE, PAUSE, SELECT_VIRTUAL_ADDRESS_SPACE, WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_ps3_abi::lv1::pause::PAUSE_MODE_R3;

use super::super::RECENT_PCS;
use super::page_table::{
    hypercall, li, lis, mr, ori, oris, read_and_send, send_r8, vas_id, DATA_PAGE, DATA_RA_HIGH,
    ESID_EA, PP_READ_WRITE, SLOT, VSID,
};
use super::*;

#[path = "from_entry_pause_ctrl_tests.rs"]
mod pause_ctrl;

/// A firmware whose lv1 `pause` handler an lv1 image read shows.
const FW_PAUSE: &str = "4.93";

/// `mfspr r3, 136`: `mfctrl r3`.
const MFCTRL_R3: u32 = 0x7C68_22A6;
/// `andis. r3, r3, CT thread 1`: CR0 EQ is clear on thread 1 only.
const ANDIS_R3_CT1: u32 = 0x7463_0000 | (CTRL_CT_THREAD1 >> 16);
/// `cmplw cr0, r3, r4`.
const CMPLW_R3_R4: u32 = 0x7C03_2040;
const LI_R3_0: u32 = 0x3860_0000;
const LI_R3_1: u32 = 0x3860_0001;
const LI_R11_PAUSE: u32 = 0x3960_0000 | PAUSE as u32;

/// `beq` to the word `words` words ahead.
const fn beq(words: u32) -> u32 {
    0x4182_0000 | (words * 4)
}

/// `bne` to the word `words` words ahead.
const fn bne(words: u32) -> u32 {
    0x4082_0000 | (words * 4)
}

/// A kernel whose entry is the reset vector, at offset 0x100, so both
/// threads run `words` from its first word.
fn shared_entry_elf(words: &[u32]) -> Vec<u8> {
    elf_image(BASE, 0, words, 0x100)
}

/// The address of word `index` of a [`shared_entry_elf`] on thread 0,
/// which starts at the ELF's entry.
fn thread0_pc(index: u64) -> u64 {
    BASE + 0x100 + 4 * index
}

/// The address of word `index` of a [`shared_entry_elf`] on thread 1,
/// which starts at its reset vector with no high address bit.
fn thread1_pc(index: u64) -> u64 {
    VECTOR_SYSTEM_RESET_THREAD1 + 4 * index
}

#[test]
fn each_thread_reads_its_own_thread_in_ct_and_both_threads_in_te() {
    let enabled = CTRL_TE_THREAD0 | CTRL_TE_THREAD1;
    let code = [
        MFCTRL_R3,
        lis(4, (CTRL_CT_THREAD0 | enabled) >> 16),
        CMPLW_R3_R4,
        beq(5),
        lis(4, (CTRL_CT_THREAD1 | enabled) >> 16),
        CMPLW_R3_R4,
        beq(3),
        B_SELF, // 7: neither value
        B_SELF, // 8: thread 0's value
        B_SELF, // 9: thread 1's value
    ];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(8), thread1_pc(9)));
}

#[test]
fn thread_1_starts_at_its_reset_vector_in_supervisor_state_with_64_bit_mode() {
    let run = run_kernel_from_entry(&kernel_elf(&[B_SELF]), FW, 50).unwrap();
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!(thread1.pc, VECTOR_SYSTEM_RESET_THREAD1);
    assert_eq!(thread1.msr, MSR_SF);
    assert_eq!(thread1.status, Some(UnitStatus::Runnable));
    assert_eq!(
        thread1.recent_pcs,
        vec![VECTOR_SYSTEM_RESET_THREAD1; RECENT_PCS]
    );
}

#[test]
fn a_thread_1_fault_stops_the_run_and_names_thread_1() {
    let code = [MFCTRL_R3, ANDIS_R3_CT1, bne(2), B_SELF, MFSPR_R0_PVR];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW, 10_000).unwrap();
    let KernelRunStop::Fault { thread, pc, .. } = run.stop else {
        panic!("the run stopped at {:?}", run.stop);
    };
    assert_eq!((thread, pc), (1, Some(thread1_pc(4))));
}

#[test]
fn a_pause_parks_its_thread_on_the_sc_while_the_other_thread_runs_on() {
    let code = [LI_R3_0, LI_R11_PAUSE, SC_1, B_SELF];
    let run = run_kernel_from_entry(&kernel_elf(&code), FW_PAUSE, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    assert_eq!(run.steps, 50);
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!(thread0.status, Some(UnitStatus::Blocked));
    assert_eq!(thread0.pc, ENTRY + 8);
    assert_eq!(thread0.last_hypercall.map(|c| c.number), Some(PAUSE));
    assert_eq!(thread1.status, Some(UnitStatus::Runnable));
}

#[test]
fn a_pause_of_both_threads_stops_the_run_with_each_threads_wake_flag() {
    // Thread 0 pauses in mode 1 and thread 1 in mode 0.
    let code = [
        MFCTRL_R3,
        ANDIS_R3_CT1,
        LI_R3_1,
        beq(2),
        LI_R3_0,
        LI_R11_PAUSE,
        SC_1,
        B_SELF,
    ];
    let run = run_kernel_from_entry(&shared_entry_elf(&code), FW_PAUSE, 10_000).unwrap();
    assert_eq!(
        run.stop,
        KernelRunStop::NoWake {
            paused: vec![
                PausedThread {
                    thread: 0,
                    decrementer_wakes: false,
                },
                PausedThread {
                    thread: 1,
                    decrementer_wakes: true,
                },
            ],
        }
    );
    let thread0 = run.thread0.expect("thread 0's position");
    let thread1 = run.thread1.expect("thread 1's position");
    assert_eq!((thread0.pc, thread1.pc), (thread0_pc(6), thread1_pc(6)));
    assert_eq!(
        (thread0.status, thread1.status),
        (Some(UnitStatus::Blocked), Some(UnitStatus::Blocked))
    );
}

#[test]
fn a_pause_in_a_mode_lv1_refuses_returns_its_status_to_the_caller_with_no_park() {
    // The caller sends the r3 that the pause returned.
    let mut code = vec![LI_R3_2, LI_R11_PAUSE, SC_1, mr(8, 3)];
    code.extend(send_r8());
    code.push(B_SELF);
    let spin = u64::try_from(code.len()).expect("a short program") - 1;
    let run = run_kernel_from_entry(&kernel_elf(&code), FW_PAUSE, 50).unwrap();
    assert_eq!(run.stop, KernelRunStop::StepCap);
    let thread0 = run.thread0.expect("thread 0's position");
    assert_eq!(thread0.status, Some(UnitStatus::Runnable));
    assert_eq!(thread0.pc, ENTRY + 4 * spin);
    assert_eq!(run.console, PAUSE_MODE_R3.to_be_bytes().to_vec());
}

/// `lwz r22, 0(r21)`, `stw r22, 0(r21)` and `cmpwi cr0, r22, 0`.
const LWZ_R22_R21: u32 = 0x82D5_0000;
const STW_R22_R21: u32 = 0x92D5_0000;
const CMPWI_R22_0: u32 = 0x2C16_0000;
/// `beq` to the word 3 words back.
const BEQ_BACK_3: u32 = 0x4182_FFF4;
/// The real page of the flag that thread 0 sets once the page table holds
/// the data page's entry.
const FLAG_PAGE: u32 = 0x1;

/// A program in which thread 1 uses the page table that thread 0 selects.
///
/// Thread 0:
/// 1. constructs and selects the address space;
/// 2. writes the data page's entry;
/// 3. sets the flag.
///
/// Thread 1:
/// 1. waits for the flag;
/// 2. maps the segment in its own SLB;
/// 3. loads and stores the data page with data translation on;
/// 4. reads the entry and sends its low halfword;
/// 5. stops at hypercall 3, which CellGov does not answer.
fn shared_page_table_program() -> Vec<u32> {
    let mut thread0 = vec![li(3, 0), li(4, 2), lis(5, 0x1014), SLDI_R5_32];
    thread0.extend(hypercall(CONSTRUCT_VIRTUAL_ADDRESS_SPACE));
    thread0.push(li(3, vas_id()));
    thread0.extend(hypercall(SELECT_VIRTUAL_ADDRESS_SPACE));
    thread0.extend([
        li(3, 0),
        li(4, SLOT),
        lis(5, VSID >> 4),
        ori(5, 5, ((VSID & 0xF) << 12) | 1),
        lis(6, DATA_RA_HIGH),
        ori(6, 6, PP_READ_WRITE),
    ]);
    thread0.extend(hypercall(WRITE_HTAB_ENTRY));
    thread0.extend([lis(21, FLAG_PAGE), li(22, 1), STW_R22_R21, B_SELF]);

    let slb_valid_high = (SLB_VALID >> 16) as u32;
    let mut thread1 = vec![lis(21, FLAG_PAGE), LWZ_R22_R21, CMPWI_R22_0, BEQ_BACK_3];
    thread1.extend([
        li(12, 0),
        oris(12, 12, ESID_EA | slb_valid_high),
        lis(13, VSID >> 4),
        ori(13, 13, (VSID & 0xF) << 12),
        0x7C00_0324 | (13 << 21) | (12 << 11),
        li(20, DATA_PAGE),
        oris(20, 20, ESID_EA),
        0x7C00_00A6 | (8 << 21),
        ori(9, 8, MSR_DR as u32),
        0x7C00_0164 | (9 << 21),
        0x4C00_012C,
        0xE800_0000 | (10 << 21) | (20 << 16),
        0xF800_0000 | (10 << 21) | (20 << 16) | 8,
        0x7C00_0164 | (8 << 21),
        0x4C00_012C,
    ]);
    thread1.extend(read_and_send());
    thread1.extend(hypercall(3));

    let skip = u32::try_from(thread0.len()).expect("a short program") + 1;
    let mut code = vec![MFCTRL_R3, ANDIS_R3_CT1, bne(skip)];
    code.extend(thread0);
    code.extend(thread1);
    code
}

#[test]
fn a_page_table_that_thread_0_selects_translates_thread_1s_accesses() {
    let elf = shared_entry_elf(&shared_page_table_program());
    let run = run_kernel_from_entry(&elf, FW_PAUSE, 10_000).unwrap();
    assert!(
        matches!(
            run.stop,
            KernelRunStop::Refused(HypercallRefusal::Unmodelled { number: 3, unit, .. })
                if unit == UnitId::new(1)
        ),
        "the run stopped at {:?}",
        run.stop
    );
    let referenced = PTE_M | u64::from(PP_READ_WRITE) | PTE_R | PTE_C;
    assert_eq!(run.console, (referenced << 48).to_be_bytes().to_vec());
}
