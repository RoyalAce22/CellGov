//! A unit with a tap installed holds to the [`PpuTap::dispatch`] contract.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use cellgov_exec::ExecutionContext;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::instruction::PpuInstruction;
use crate::state::PpuState;

const STORE_BUFFER_CAPACITY: usize = 64;

#[derive(Default)]
struct Pcs(RefCell<Vec<u64>>);

impl PpuTap for Pcs {
    fn dispatch(&self, _insn: &PpuInstruction, state: &PpuState) {
        self.0.borrow_mut().push(state.pc);
    }
}

fn place_insn(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

/// `stw r3, off(r4)`.
fn stw_r3_off_r4(off: u16) -> u32 {
    (36 << 26) | (3 << 21) | (4 << 16) | u32::from(off)
}

#[test]
fn every_dispatch_is_reported_in_order() {
    let mut mem = GuestMemory::new(0x1000);
    place_insn(&mut mem, 0, stw_r3_off_r4(0));
    place_insn(&mut mem, 4, stw_r3_off_r4(4));
    place_insn(&mut mem, 8, 0x4400_0002); // sc
    let pcs = Rc::new(Pcs::default());
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(4, 0x800);
    unit.set_tap(pcs.clone());

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(*pcs.0.borrow(), vec![0, 4, 8]);
}

#[test]
fn a_store_the_full_buffer_retries_is_reported_twice() {
    const RETRY_PC: u64 = (STORE_BUFFER_CAPACITY * 4) as u64;
    let mut mem = GuestMemory::new(0x1000);
    for i in 0..=STORE_BUFFER_CAPACITY {
        place_insn(&mut mem, i * 4, stw_r3_off_r4((i * 4) as u16));
    }
    let pcs = Rc::new(Pcs::default());
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(4, 0x800);
    unit.set_tap(pcs.clone());

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(200), &ctx, &mut effects);
    assert_eq!(unit.state().pc, RETRY_PC);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(1), &ctx, &mut effects);

    let seen = pcs.0.borrow();
    assert_eq!(seen.len(), STORE_BUFFER_CAPACITY + 2);
    assert_eq!(seen[STORE_BUFFER_CAPACITY..], [RETRY_PC, RETRY_PC]);
}

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

#[test]
fn a_fault_rolls_back_the_batch_but_not_its_reports() {
    let mut mem = GuestMemory::new(0x1000);
    place_insn(&mut mem, 0, addi_inc(3));
    place_insn(&mut mem, 4, addi_inc(3));
    place_insn(&mut mem, 8, (32 << 26) | (6 << 21) | (5 << 16)); // lwz r6, 0(r5)
    let pcs = Rc::new(Pcs::default());
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, 0xFFFF_0000);
    unit.set_tap(pcs.clone());

    let ctx = ExecutionContext::new(&mem);
    let r = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(unit.state().pc, 0, "the unit rewinds to batch entry");
    assert_eq!(*pcs.0.borrow(), vec![0, 4, 8]);
}

#[test]
fn the_second_slot_of_a_fused_pair_is_not_reported() {
    let lwz: u32 = (32 << 26) | (3 << 21) | 128; // lwz r3, 128(0)
    let cmpwi: u32 = (11 << 26) | (3 << 16) | 5; // cmpwi r3, 5
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, lwz);
    place_insn(&mut mem, 4, cmpwi);
    let mut words = [0u8; 8];
    words[..4].copy_from_slice(&lwz.to_be_bytes());
    words[4..].copy_from_slice(&cmpwi.to_be_bytes());
    let shadow = shadow::PredecodedShadow::build(0, &words);
    assert_eq!(
        shadow.get(4),
        Some(PpuInstruction::Consumed),
        "precondition: the pair fuses"
    );
    let pcs = Rc::new(Pcs::default());
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.set_tap(pcs.clone());

    let ctx = ExecutionContext::new(&mem);
    let r = unit.run_until_yield(Budget::new(2), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 8, "both slots retired");
    assert_eq!(*pcs.0.borrow(), vec![0]);
}
