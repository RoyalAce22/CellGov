//! A PPU branch into a Raw SPU's local-store window, checked against the
//! `ppu_code_call_raw_spu` console capture.
//!
//! On the console, a child thread that branches to two PPU instructions
//! stored in Raw SPU 0's local store ends at the branch. A child that
//! branches to the same two instructions in the probe's own code returns.
//! CellGov refuses a fetch from a window with the instruction storage
//! interrupt.

use cellgov_effects::FaultKind;
use cellgov_mem::{PageSize, Region, RegionAccess};
use cellgov_ppu::{PpuFaultClass, FAULT_INSTRUCTION_STORAGE};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::lv2::errno::CELL_ESRCH;
use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::syscall as sc;

use crate::span_harness::{call, exit, li, oris, payload, run, stw, word, OUT};

/// The done marker, "RAWX".
const DONE: u32 = 0x5241_5758;
/// The value the probe writes for a word it did not reach.
const SKIP: u32 = 0xDEAD_D00D;
/// The value the probe's child passes to its thread exit.
const CHILD_EXIT: u32 = 0xE5;

/// `li r3,0x77`, the first of the two instructions both calls reach.
const LI_R3_77: u32 = 0x3860_0077;
const BLR: u32 = 0x4E80_0020;
const MTCTR_R5: u32 = 0x7CA9_03A6;
const BCTRL: u32 = 0x4E80_0421;

/// Memory as the boot gives it: main memory and the reserved SPU range,
/// which is caching-inhibited and holds one window here.
fn regions() -> Vec<Region> {
    vec![
        Region::new(0, 0x2_0000, "main", PageSize::Page64K),
        Region::with_access(
            raw_window::BASE,
            raw_window::STRIDE as usize,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )
        .with_caching_inhibited(),
    ]
}

/// The probe's setup, then a branch to Raw SPU 0's window, then
/// `sys_process_exit`. r14 and r15 hold the call return codes.
fn program() -> Vec<u32> {
    let mut p = Vec::new();
    call(&mut p, sc::SPU_INITIALIZE, &[li(3, 6), li(4, 1)], 14);
    call(
        &mut p,
        sc::RAW_SPU_CREATE,
        &[li(3, OUT as i16), li(4, 0)],
        15,
    );
    p.push(li(5, 0));
    p.push(oris(5, 5, (raw_window::BASE >> 16) as u16));
    for (i, insn) in [LI_R3_77, BLR].iter().enumerate() {
        word(&mut p, 6, *insn);
        p.push(stw(6, 4 * i as i16, 5));
    }
    p.push(li(3, 0));
    p.push(MTCTR_R5);
    p.push(BCTRL);
    exit(&mut p);
    p
}

#[test]
fn a_branch_into_a_raw_spu_window_takes_the_instruction_storage_interrupt_as_on_the_console() {
    let w = payload("ppu_code_call_raw_spu", 24);
    assert_eq!(w[0..2], [DONE, 0], "the main thread finished");
    assert_eq!(w[2..6], [0, 0, 0, 0], "Raw SPU 0 and its destroy");
    assert_eq!(w[6..8], [LI_R3_77, BLR], "the local store holds both words");
    assert_eq!(
        w[8..16],
        [0, 0, 1, 1, 0x77, 0, 0, CHILD_EXIT],
        "the control call returns"
    );
    assert_eq!(w[16..19], [0, 0, 1], "case 2: create, start, reached");
    assert_eq!(w[19], 0, "case 2: the call does not return");
    assert_eq!(w[21], CELL_ESRCH.code, "case 2: the child is gone");
    assert_eq!([w[20], w[22], w[23]], [SKIP; 3], "case 2: no r3, no join");

    let ran = run(regions(), &program());
    assert_eq!(ran.gpr()[14..16], [0, 0], "initialize and the create");
    assert_ne!(ran.gpr()[3], 0x77, "the window's li never ran");
    let (unit, kind) = ran.first_fault.expect("the branch faults");
    assert_eq!(unit, ran.ppu, "the PPU faults");
    let FaultKind::Guest(code) = kind else {
        panic!("{kind:?}");
    };
    assert_eq!(
        PpuFaultClass::of_code(code),
        Some(PpuFaultClass::InstructionStorage)
    );
    assert_eq!(
        code,
        FAULT_INSTRUCTION_STORAGE | (irq::SRR1_FETCH_NO_EXECUTE >> 16) as u32,
        "the window is mapped, so the fetch is refused as no-execute"
    );
}
