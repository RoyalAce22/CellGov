use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    DID_UPDATE_INTERRUPT_MASK, SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::event_port::game_os_port_calls;
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_ps3_abi::lv1::interrupt_mask::{EXTERNAL_INTERRUPT_VECTOR, MEDIATED_UNDELIVERED_PC};

use super::super::KernelRun;
use super::page_table::{li, lis, ori, oris};
use super::*;

/// A firmware whose lv1 return path and external interrupt handler an
/// lv1 image read shows.
const FW_MEDIATED: &str = "4.93";

/// A port id that lv1 gives a call of thread 0.
const PORT: u32 = 3;

/// Thread 0's interrupt state bitmap, in the real mode region and past
/// the kernel's segment.
const BITMAP_HIGH: u32 = 0x1;

const LI_R3_1: u32 = 0x3860_0001;
/// `std r5, 0(r4)`.
const STD_R5_R4: u32 = 0xF8A4_0000;
/// `mfmsr r3`.
const MFMSR_R3: u32 = 0x7C60_00A6;
/// `mtmsrd r3, 0`.
const MTMSRD_R3: u32 = 0x7C60_0164;
/// `mtspr 22, r3`: `mtdec r3`.
const MTDEC_R3: u32 = 0x7C76_03A6;
/// `ori r0, r0, 0`.
const NOP: u32 = 0x6000_0000;
/// `mfspr r4, 26`: `mfsrr0 r4`.
const MFSRR0_R4: u32 = 0x7C9A_02A6;
/// `mfspr r3, 27`: `mfsrr1 r3`.
const MFSRR1_R3: u32 = 0x7C7B_02A6;
/// `cmpd r4, r5`.
const CMPD_R4_R5: u32 = 0x7C24_2800;
/// `andis. r0, r3, 0x4020`: SRR1 bits 33 and 42.
const ANDIS_R0_R3_SRR1_BITS: u32 = 0x7460_4020;
/// `xoris r0, r0, 0x4020`.
const XORIS_R0_R0_SRR1_BITS: u32 = 0x6C00_4020;
/// `cmpdi r0, 0`.
const CMPDI_R0_0: u32 = 0x2C20_0000;

/// The handler's word that faults when SRR0 and SRR1 are as expected.
/// The next word faults for another SRR0, and the word after it for an
/// SRR1 without bits 33 and 42.
const HANDLER_PASS: u64 = EXTERNAL_INTERRUPT_VECTOR + 4 * 15;

/// The `construct_event_receive_port` call of thread 0 whose port id is
/// [`PORT`] (`GAME_OS_PORT_CALLS`).
fn port_sc() -> u64 {
    game_os_port_calls(FW_MEDIATED)
        .expect("the 4.93 port calls")
        .iter()
        .find(|call| call.thread == 0 && call.id == u64::from(PORT))
        .expect("thread 0's call for the port")
        .sc
}

/// The word index of offset `offset` of the kernel's segment, whose
/// words start at offset 0x100.
const fn index(offset: u64) -> usize {
    ((offset - 0x100) / 4) as usize
}

/// The address of the word at index `word`.
fn address(word: usize) -> u64 {
    BASE + 0x100 + 4 * word as u64
}

/// `b` from the word at `from` to the word at `to`, both word indexes.
fn b_to(from: usize, to: usize) -> u32 {
    let disp = (to as i64 - from as i64) * 4;
    0x4800_0000 | (disp as u32 & 0x03FF_FFFC)
}

/// `bne` from the word at `from` to the word at `to`, both word indexes.
fn bne_to(from: usize, to: usize) -> u32 {
    let disp = (to as i64 - from as i64) * 4;
    0x4082_0000 | (disp as u32 & 0xFFFC)
}

/// Sets MSR\[EE\] and the MSR bits `bits`: `mfmsr`, `ori` and `mtmsrd`.
fn set_msr(bits: u32) -> [u32; 3] {
    [MFMSR_R3, ori(3, 3, 0x8000 | bits), MTMSRD_R3]
}

/// Stores 0 in thread 0's mask doubleword 0.
fn clear_mask() -> [u32; 4] {
    [
        lis(4, BITMAP_HIGH),
        ori(4, 4, IRQ_MASK_OFFSET as u32),
        li(5, 0),
        STD_R5_R4,
    ]
}

/// What the kernel does after its send, with MSR\[EE\] still 0.
enum After {
    /// Two other instructions, then the move that sets EE and `bits`.
    SetEe(u32),
    /// The store of 0 in its mask, then the move that sets EE.
    ClearMask,
    /// The store of 0 in its mask, hypercall 31 for its own thread, then
    /// the move that sets EE.
    ClearMaskAndTell,
    /// A branch to lv1's slot 0x28B8 less 12, whose words set EE.
    AtSlot,
}

/// A kernel whose thread 0 raises its own external interrupt through lv1
/// with MSR\[EE\] = 0:
/// 1. it stores plug 0's bit in mask doubleword 0, and registers the
///    bitmap;
/// 2. it constructs port 3 from the `sc` at [`port_sc`];
/// 3. it connects the port to plug 0 of thread 0;
/// 4. it sends an event to the port, then does `after`.
///
/// The word after the move that sets EE faults. Returns the image and
/// the address of that word.
///
/// The handler at the vector sets EE again, then faults at
/// [`HANDLER_PASS`] when SRR0 is that address and SRR1 holds bits 33 and
/// 42. A request that the take left would interrupt the handler.
fn requesting_kernel(after: After) -> (Vec<u8>, u64) {
    let vector = index(EXTERNAL_INTERRUPT_VECTOR);
    let construct_sc = index(port_sc() - BASE);
    let mut words = vec![B_SELF; construct_sc + 32];

    let setup = [
        lis(5, 0x8000),
        SLDI_R5_32,
        lis(4, BITMAP_HIGH),
        ori(4, 4, IRQ_MASK_OFFSET as u32),
        STD_R5_R4,
        LI_R3_1,
        li(4, 0),
        lis(5, BITMAP_HIGH),
        li(11, CONFIGURE_IRQ_STATE_BITMAP as u32),
        SC_1,
        // DEC = 0x7FFF_FFFF, so no decrementer interrupt comes first.
        lis(3, 0x7FFF),
        ori(3, 3, 0xFFFF),
        MTDEC_R3,
    ];
    words[1..=setup.len()].copy_from_slice(&setup);
    let branch = setup.len() + 1;
    words[branch] = b_to(branch, construct_sc - 1);

    let mut raise = vec![li(11, CONSTRUCT_EVENT_RECEIVE_PORT as u32), SC_1];
    raise.extend([li(3, 1), li(4, 0), li(5, 0), li(6, PORT), li(7, 0)]);
    raise.extend([li(11, CONNECT_IRQ_PLUG_EXT as u32), SC_1]);
    raise.extend([li(3, PORT), li(11, SEND_EVENT_LOCALLY as u32), SC_1]);
    let start = construct_sc - 1;
    let fault = match after {
        After::SetEe(bits) => {
            raise.extend([NOP, NOP]);
            raise.extend(set_msr(bits));
            address(start + raise.len())
        }
        After::ClearMask => {
            raise.extend(clear_mask());
            raise.extend(set_msr(0));
            address(start + raise.len())
        }
        After::ClearMaskAndTell => {
            raise.extend(clear_mask());
            raise.extend([LI_R3_1, li(4, 0)]);
            raise.extend([li(11, DID_UPDATE_INTERRUPT_MASK as u32), SC_1]);
            raise.extend(set_msr(0));
            address(start + raise.len())
        }
        After::AtSlot => {
            // `ba` to the slot less 12.
            raise.push(0x4800_0002 | (MEDIATED_UNDELIVERED_PC as u32 - 12));
            let slot = index(MEDIATED_UNDELIVERED_PC);
            words[slot - 3..slot].copy_from_slice(&set_msr(0));
            words[slot] = MFSPR_R0_PVR;
            MEDIATED_UNDELIVERED_PC
        }
    };
    raise.push(MFSPR_R0_PVR);
    words[start..start + raise.len()].copy_from_slice(&raise);

    let low = fault as u32;
    let mut handler = set_msr(0).to_vec();
    handler.extend([
        MFSRR0_R4,
        lis(5, 0x8000),
        SLDI_R5_32,
        oris(5, 5, low >> 16),
        ori(5, 5, low & 0xFFFF),
        CMPD_R4_R5,
        bne_to(9, 16),
        MFSRR1_R3,
        ANDIS_R0_R3_SRR1_BITS,
        XORIS_R0_R0_SRR1_BITS,
        CMPDI_R0_0,
        bne_to(14, 17),
        MFSPR_R0_PVR,
        MFSPR_R0_PVR,
        MFSPR_R0_PVR,
    ]);
    words[vector..vector + handler.len()].copy_from_slice(&handler);
    // The setup, the handler, lv1's slot and the raise hold disjoint words.
    let slot = index(MEDIATED_UNDELIVERED_PC);
    assert!(
        branch < vector && vector + handler.len() <= slot - 3 && slot < start,
        "the kernel's words overlap"
    );

    (elf_image(BASE, 0, &words, 0x104), fault)
}

/// The pc of the fault that stopped `run`.
fn fault_pc(run: &KernelRun) -> u64 {
    let KernelRunStop::Fault { thread, pc, .. } = run.stop else {
        panic!("the run stopped at {:?} {:x?}", run.stop, run.thread0);
    };
    assert_eq!(thread, 0);
    pc.expect("the fault's pc")
}

/// The refusal text of the mediated take that stopped `run`, and the pc
/// it names.
fn take_refusal(run: &KernelRun) -> (u64, &'static str) {
    let KernelRunStop::Refused(HypercallRefusal::MediatedExternal { pc, what, .. }) = run.stop
    else {
        panic!("the run stopped at {:?} {:x?}", run.stop, run.thread0);
    };
    (pc, what)
}

/// The send's return finds plug 0 in the shadow and the mask with the
/// saved MSR\[EE\] = 0, so lv1 sets LPCR\[MER\]
/// [OFW-4.93 lv1 a:0x191C-0x1928]. The thread continues to run until its
/// move sets EE, and then takes the interrupt at 0x500.
///
/// SRR0 is the word after the move, and SRR1 holds bits 33 and 42, as
/// lv1 gives the saved HSRR0 and HSRR1 [OFW-4.93 lv1 a:0x188C-0x18C0]
/// [CBE-Handbook p:277 s:9.8]. lv1's handler reloads the LPCR, which
/// clears LPCR\[MER\] [OFW-4.93 lv1 a:0x1A5C], so the handler's own EE
/// takes nothing.
#[test]
fn an_ee_off_return_requests_an_interrupt_that_the_thread_takes_once_it_sets_ee() {
    let (kernel, _) = requesting_kernel(After::SetEe(0));
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(fault_pc(&run), HANDLER_PASS);
    let thread0 = run.thread0.expect("thread 0's position");
    let entries = thread0
        .recent_pcs
        .iter()
        .filter(|&&pc| pc == EXTERNAL_INTERRUPT_VECTOR)
        .count();
    assert_eq!(entries, 1, "recent pcs {:x?}", thread0.recent_pcs);
}

/// Hypercall 31's return finds no shared bit and reloads the LPCR, which
/// clears LPCR\[MER\] [OFW-4.93 lv1 a:0x1008]. So the move that sets EE
/// takes nothing, and the next word faults.
#[test]
fn a_later_return_that_finds_no_bit_ends_the_request() {
    let (kernel, after_move) = requesting_kernel(After::ClearMaskAndTell);
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(fault_pc(&run), after_move);
}

/// The mask lost its bit with no hypercall, so LPCR\[MER\] stays. lv1's
/// handler finds no bit and enters no vector
/// [OFW-4.93 lv1 a:0x1A84-0x1AD4].
#[test]
fn a_take_after_the_mask_lost_its_bit_stops_the_run() {
    let (kernel, after_move) = requesting_kernel(After::ClearMask);
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(
        take_refusal(&run),
        (
            after_move,
            "a take whose status shadow and mask share no bit, for which lv1's handler enters \
             no vector"
        )
    );
}

/// `mtmsrd` copies MSR bit 56 [PPC-Book3 p:22 s:3.4]. lv1 clears it in the
/// vector's MSR [OFW-4.93 lv1 a:0x1898], and the PPU's entry keeps it.
#[test]
fn a_take_with_msr_bit_56_stops_the_run() {
    let (kernel, after_move) = requesting_kernel(After::SetEe(0x80));
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(
        take_refusal(&run),
        (
            after_move,
            "a take whose MSR holds a bit of 48 to 63 that lv1 clears and the PPU's interrupt \
             entry keeps"
        )
    );
}

/// lv1 keeps LPCR\[MER\] for a saved pc of 0x28B8
/// [OFW-4.93 lv1 a:0x188C-0x18A8].
#[test]
fn a_take_at_lv1_s_slot_stops_the_run() {
    let (kernel, slot) = requesting_kernel(After::AtSlot);
    let run = run_kernel_from_entry(&kernel, FW_MEDIATED, 2_000).unwrap();
    assert_eq!(
        take_refusal(&run),
        (
            slot,
            "a take at lv1's trampoline slot 0x28B8, for which lv1's handler enters no vector"
        )
    );
}
