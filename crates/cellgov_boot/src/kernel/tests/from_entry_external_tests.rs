use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::event_port::game_os_port_calls;
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_ps3_abi::lv1::interrupt_mask::EXTERNAL_INTERRUPT_VECTOR;

use super::page_table::{li, lis, ori};
use super::*;

/// A firmware whose lv1 return path an lv1 image read shows.
const FW_EXTERNAL: &str = "4.93";

/// The `construct_event_receive_port` call of thread 0 whose port id is
/// [`PORT`] (`GAME_OS_PORT_CALLS`).
fn port_sc() -> u64 {
    game_os_port_calls(FW_EXTERNAL)
        .expect("the 4.93 port calls")
        .iter()
        .find(|call| call.thread == 0 && call.id == u64::from(PORT))
        .expect("thread 0's call for the port")
        .sc
}

/// A port id that lv1 gives a call of thread 0.
const PORT: u32 = 3;

/// Thread 0's interrupt state bitmap, in the real mode region and past
/// the kernel's segment.
const BITMAP_HIGH: u32 = 0x1;

/// `std r5, 0(r4)`.
const STD_R5_R4: u32 = 0xF8A4_0000;
/// `mfmsr r3`.
const MFMSR_R3: u32 = 0x7C60_00A6;
/// `mtmsrd r3, 0`.
const MTMSRD_R3: u32 = 0x7C60_0164;
/// `mtspr 22, r3`: `mtdec r3`.
const MTDEC_R3: u32 = 0x7C76_03A6;

/// The word index of offset `offset` of the kernel's segment, whose
/// words start at offset 0x100.
const fn index(offset: u64) -> usize {
    ((offset - 0x100) / 4) as usize
}

/// `b` from the word at `from` to the word at `to`, both word indexes.
fn b_to(from: usize, to: usize) -> u32 {
    let disp = (to as i64 - from as i64) * 4;
    0x4800_0000 | (disp as u32 & 0x03FF_FFFC)
}

/// Sets MSR\[EE\] in place: `mfmsr`, `ori` and `mtmsrd`.
fn set_ee() -> [u32; 3] {
    [MFMSR_R3, ori(3, 3, 0x8000), MTMSRD_R3]
}

/// A kernel whose thread 0 raises its own external interrupt through lv1:
/// 1. it stores plug 0's bit in mask doubleword 0, and registers the
///    bitmap;
/// 2. it constructs port 3 from the `sc` at [`port_sc`];
/// 3. it connects the port to plug 0 of thread 0, and sets MSR\[EE\];
/// 4. it sends an event to the port.
///
/// The word after the send faults. The handler at the vector sets EE
/// again and then faults; with the exception still raised, the thread
/// takes the interrupt again ahead of that fault.
fn raising_kernel() -> Vec<u8> {
    let vector = index(EXTERNAL_INTERRUPT_VECTOR);
    let construct_sc = index(port_sc() - BASE);
    let mut words = vec![B_SELF; construct_sc + 16];

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

    let mut handler = set_ee().to_vec();
    handler.push(MFSPR_R0_PVR);
    words[vector..vector + handler.len()].copy_from_slice(&handler);

    let mut raise = vec![li(11, CONSTRUCT_EVENT_RECEIVE_PORT as u32), SC_1];
    raise.extend([li(3, 1), li(4, 0), li(5, 0), li(6, PORT), li(7, 0)]);
    raise.extend([li(11, CONNECT_IRQ_PLUG_EXT as u32), SC_1]);
    raise.extend(set_ee());
    raise.extend([li(3, PORT), li(11, SEND_EVENT_LOCALLY as u32), SC_1]);
    raise.push(MFSPR_R0_PVR);
    let start = construct_sc - 1;
    words[start..start + raise.len()].copy_from_slice(&raise);

    elf_image(BASE, 0, &words, 0x104)
}

const LI_R3_1: u32 = 0x3860_0001;

/// The send's return finds plug 0 set in the status shadow and in the
/// mask, with the saved MSR\[EE\] set, so lv1 enters the vector
/// [OFW-4.93 lv1 a:0x0FC4-0x101C] [OFW-4.93 lv1 a:0x18E0-0x1914]. The PPU
/// takes the external interrupt there, and the runtime clears the
/// exception, so the handler's own EE does not take it again.
#[test]
fn a_raised_plug_enters_the_external_vector_once_and_the_exception_ends() {
    let run = run_kernel_from_entry(&raising_kernel(), FW_EXTERNAL, 2_000).unwrap();
    let handler_fault = EXTERNAL_INTERRUPT_VECTOR + 4 * 3;
    let KernelRunStop::Fault { thread, pc, .. } = run.stop else {
        panic!("the run stopped at {:?} {:x?}", run.stop, run.thread0);
    };
    assert_eq!((thread, pc), (0, Some(handler_fault)));
    let thread0 = run.thread0.expect("thread 0's position");
    let entries = thread0
        .recent_pcs
        .iter()
        .filter(|&&pc| pc == EXTERNAL_INTERRUPT_VECTOR)
        .count();
    assert_eq!(entries, 1, "recent pcs {:x?}", thread0.recent_pcs);
}
