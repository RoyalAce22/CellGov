//! A word that is not an SPU instruction stops the SPU as an invalid
//! instruction; an instruction CellGov does not implement is a CellGov
//! refusal that names it.

use crate::stop::{SpuStop, SpuStopKind};
use crate::SpuExecutionUnit;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, StopRegisters, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP;
use cellgov_time::Budget;

fn run_word_at_0x20(word: u32) -> (SpuExecutionUnit, ExecutionStepResult) {
    let mut unit = SpuExecutionUnit::new(UnitId::new(1));
    unit.state_mut().ls[0x20..0x24].copy_from_slice(&word.to_be_bytes());
    unit.state_mut().pc = 0x20;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(10), &ExecutionContext::new(&mem), &mut effects);
    (unit, result)
}

// [CBEA p:33 s:2.1.2] an SPU that meets an invalid instruction halts and records it in its status register.
// [CBEA p:93 s:8.5.2] I (bit 26): invalid instruction detected, SPU stopped.
#[test]
fn an_unassigned_word_stops_the_unit_as_an_invalid_instruction_at_that_word() {
    let (unit, result) = run_word_at_0x20(0x9000_0000);
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(result.fault, None);
    assert_eq!(unit.status(), UnitStatus::Finished);
    assert_eq!(
        unit.state().stop,
        Some(SpuStop {
            kind: SpuStopKind::InvalidInstruction,
            code: 0,
            npc: 0x20,
        })
    );
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: 0x0000_0020,
            npc: 0x20
        })
    );
}

// [CBE-Handbook p:766 s:B.1 Table B-1] the CBE's SPU instruction table has no double-precision compares.
#[test]
fn an_optional_instruction_the_cbe_lacks_is_an_invalid_instruction() {
    let dfceq = SPU_OPCODE_MAP
        .iter()
        .find(|row| row.mnemonic == "dfceq")
        .expect("dfceq row");
    assert!(!dfceq.on_cbe);
    let (unit, result) = run_word_at_0x20(dfceq.canonical_word());
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(
        unit.state().stop.map(|stop| stop.kind),
        Some(SpuStopKind::InvalidInstruction)
    );
}

/// Needs a row the decoder has no arm for; the first such row stands in.
#[test]
fn an_unimplemented_instruction_is_a_refusal_that_names_it() {
    let (index, row) = SPU_OPCODE_MAP
        .iter()
        .enumerate()
        .find(|(_, row)| row.on_cbe && crate::decode::decode(row.canonical_word()).is_err())
        .expect("a CBE instruction without a decode arm");
    let (unit, result) = run_word_at_0x20(row.canonical_word());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(unit.state().stop, None);
    let Some(FaultKind::Guest(code)) = result.fault else {
        panic!("expected a guest fault, got {:?}", result.fault);
    };
    assert_eq!(code & 0xFFFF, index as u32);
    let described = crate::describe_guest_fault(code).expect("an SPU class");
    assert!(
        described.contains(&format!("({})", row.mnemonic)),
        "{described}"
    );
}
