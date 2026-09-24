//! An address-bearing SPU fault carries its address whole in
//! `faulting_ea`.

use crate::fault_codes::{
    FAULT_DETAIL_MASK, FAULT_LS_OUT_OF_RANGE, FAULT_MFC_GET_UNRESOLVED, FAULT_MFC_READ_UNRESOLVED,
};
use crate::SpuExecutionUnit;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GET, MFC_GETLLAR, MFC_PUT};
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// An address no region backs, with its low half clear so the masked
/// detail alone cannot name it.
const UNMAPPED_EA: u64 = 0x9_0000;

/// A local-store address past the 16-bit detail half but inside the
/// store, so a transfer of `OVERRUN_BYTES` from it escapes the store.
const HIGH_LSA: u32 = 0x3_FF00;
const OVERRUN_BYTES: u32 = 0x200;

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> [u8; 4] {
    (0x081u32 << 23 | (imm << 7) | rt).to_be_bytes()
}

/// `wrch $chN, $rt`.
fn wrch(channel: u8, rt: u32) -> [u8; 4] {
    (0x10Du32 << 21 | (u32::from(channel) << 7) | rt).to_be_bytes()
}

/// A unit whose local store holds `il $10, cmd; wrch $ch21, $10` and
/// whose MFC channels name `ea`, `lsa` and `size`.
fn unit_issuing(cmd: u32, ea: u64, lsa: u32, size: u32) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    s.channels.mfc_lsa = lsa;
    s.channels.mfc_eah = (ea >> 32) as u32;
    s.channels.mfc_eal = ea as u32;
    s.channels.mfc_size = size;
    s.channels.mfc_tag_id = 0;
    s.ls[0..4].copy_from_slice(&il(10, cmd));
    s.ls[4..8].copy_from_slice(&wrch(MFC_CMD, 10));
    unit
}

fn run_once(unit: &mut SpuExecutionUnit, mem: &GuestMemory) -> ExecutionStepResult {
    let ctx = ExecutionContext::new(mem);
    let mut effects: Vec<Effect> = Vec::new();
    unit.run_until_yield(Budget::new(100), &ctx, &mut effects)
}

fn class_of(result: &ExecutionStepResult) -> u32 {
    match result.fault {
        Some(FaultKind::Guest(code)) => code & !FAULT_DETAIL_MASK,
        other => panic!("expected a guest fault, got {other:?}"),
    }
}

fn detail_of(result: &ExecutionStepResult) -> u32 {
    match result.fault {
        Some(FaultKind::Guest(code)) => code & FAULT_DETAIL_MASK,
        other => panic!("expected a guest fault, got {other:?}"),
    }
}

#[test]
fn a_refused_getllar_carries_the_line_address_whole() {
    let mem = GuestMemory::new(MEM_BYTES);
    let mut unit = unit_issuing(MFC_GETLLAR, UNMAPPED_EA, 0x200, 128);
    let result = run_once(&mut unit, &mem);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(class_of(&result), FAULT_MFC_READ_UNRESOLVED);
    assert_eq!(
        detail_of(&result),
        0,
        "the detail half lost the address, which sits above it",
    );
    assert_eq!(result.local_diagnostics.faulting_ea, Some(UNMAPPED_EA));
    assert_eq!(result.local_diagnostics.pc, Some(4), "the wrch");
}

#[test]
fn a_refused_parked_get_carries_the_effective_address_whole() {
    let mem = GuestMemory::new(MEM_BYTES);
    let mut unit = unit_issuing(MFC_GET, UNMAPPED_EA, 0x200, 64);
    let issued = run_once(&mut unit, &mem);
    assert_eq!(
        issued.yield_reason,
        YieldReason::DmaSubmitted,
        "the premise: the get parks"
    );

    let performed = run_once(&mut unit, &mem);
    assert_eq!(performed.yield_reason, YieldReason::Fault);
    assert_eq!(class_of(&performed), FAULT_MFC_GET_UNRESOLVED);
    assert_eq!(
        performed.local_diagnostics.faulting_ea,
        Some(UNMAPPED_EA),
        "the detail half carries the tag id, so the address has nowhere \
         else to go",
    );
}

#[test]
fn a_put_from_past_local_store_carries_the_local_store_address_whole() {
    let mem = GuestMemory::new(MEM_BYTES);
    let mut unit = unit_issuing(MFC_PUT, 0x1000, HIGH_LSA, OVERRUN_BYTES);
    let result = run_once(&mut unit, &mem);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(class_of(&result), FAULT_LS_OUT_OF_RANGE);
    assert_eq!(
        detail_of(&result),
        HIGH_LSA & FAULT_DETAIL_MASK,
        "the detail half holds the address modulo 64 KB",
    );
    assert_eq!(
        result.local_diagnostics.faulting_ea,
        Some(u64::from(HIGH_LSA)),
        "and the diagnostics hold all 18 bits",
    );
}

#[test]
fn a_load_past_a_short_local_store_carries_its_operand_whole() {
    // lqa rt=3, imm=0x7FFE: 0x7FFE << 2 = 0x1FFF8, past a 64 KB store.
    let raw = (0x061u32 << 23) | 3 | ((0x7FFEu32 & 0xFFFF) << 7);
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    unit.state_mut().ls.truncate(0x1_0000);
    unit.state_mut().ls[0..4].copy_from_slice(&raw.to_be_bytes());
    let mem = GuestMemory::new(16);
    let result = run_once(&mut unit, &mem);

    assert_eq!(class_of(&result), FAULT_LS_OUT_OF_RANGE);
    assert_eq!(result.local_diagnostics.faulting_ea, Some(0x1_FFF8));
}

/// The fetch address is the program counter, which `pc` already carries.
#[test]
fn a_fetch_past_local_store_carries_its_pc_and_no_access_address() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    unit.state_mut().pc = 0x3_FFFC;
    // Shorten the store so the fetch at the pc escapes it.
    unit.state_mut().ls.truncate(0x3_FF00);
    let mem = GuestMemory::new(16);
    let result = run_once(&mut unit, &mem);

    assert_eq!(class_of(&result), FAULT_LS_OUT_OF_RANGE);
    assert_eq!(result.local_diagnostics.pc, Some(0x3_FFFC));
    assert_eq!(result.local_diagnostics.faulting_ea, None);
}

#[test]
fn an_spu_fault_populates_no_register_dump() {
    let mem = GuestMemory::new(MEM_BYTES);
    let mut unit = unit_issuing(MFC_GETLLAR, UNMAPPED_EA, 0x200, 128);
    let result = run_once(&mut unit, &mem);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(result.local_diagnostics.fault_regs, None);
}
