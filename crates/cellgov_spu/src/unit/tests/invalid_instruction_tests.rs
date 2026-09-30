//! A word that is not an SPU instruction, or is an optional instruction
//! the CBE does not provide, stops the SPU as an invalid instruction; an
//! instruction CellGov does not implement is a CellGov refusal that names it.

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

// [SPU-ISA p:226 s:9] dfceq, [SPU-ISA p:227 s:9] dfcmeq, [SPU-ISA p:228 s:9] dfcgt, [SPU-ISA p:229 s:9] dfcmgt and [SPU-ISA p:230 s:9] dftsv are optional in ISA 1.2.
// [CBE-Handbook p:765 s:B.1] through [CBE-Handbook p:767 s:B.1]: the CBE's SPU instruction table lists none of the five.
// [CBEA p:34 s:2.2.3] an optional instruction the implementation does not provide invokes the illegal-instruction handler.
#[test]
fn each_optional_double_compare_is_an_invalid_instruction_on_the_cbe() {
    for mnemonic in ["dfceq", "dfcmeq", "dfcgt", "dfcmgt", "dftsv"] {
        let row = SPU_OPCODE_MAP
            .iter()
            .find(|row| row.mnemonic == mnemonic)
            .expect("a row");
        assert!(!row.on_cbe, "{mnemonic}");
        // Every operand bit set, so a field-reading arm would see them.
        let word = row.canonical_word() | (u32::MAX >> row.width);
        assert_eq!(
            crate::decode::decode(word),
            Err(crate::instruction::SpuDecodeError::AbsentOnCbe {
                raw: word,
                mnemonic
            }),
            "{mnemonic}"
        );
        let (unit, result) = run_word_at_0x20(word);
        assert_eq!(result.yield_reason, YieldReason::Finished, "{mnemonic}");
        assert_eq!(result.fault, None, "{mnemonic}");
        assert_eq!(
            unit.state().stop,
            Some(SpuStop {
                kind: SpuStopKind::InvalidInstruction,
                code: 0,
                npc: 0x20,
            }),
            "{mnemonic}"
        );
        assert_eq!(unit.status(), UnitStatus::Finished, "{mnemonic}");
        assert_eq!(
            unit.stop_registers(),
            Some(StopRegisters {
                status: 0x0000_0020,
                npc: 0x20
            }),
            "{mnemonic} sets I"
        );
        assert_eq!(
            unit.state().regs,
            [[0; 16]; 128],
            "{mnemonic} wrote no register"
        );
    }
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
