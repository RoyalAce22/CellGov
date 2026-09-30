//! An opcode the SPU queue does not accept queues an invalid command
//! that names why, and a defined command the model does not run faults.

use crate::fault_codes::FAULT_UNSUPPORTED_MFC_CMD;
use crate::SpuExecutionUnit;
use cellgov_dma::MfcCommandError;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_SPU_QUEUE_DEPTH};
use cellgov_time::Budget;

const UNIT: u64 = 7;

/// `ilhu rt, imm` -- the immediate lands in the upper halfword.
fn ilhu(rt: u32, imm: u32) -> u32 {
    0x082 << 23 | ((imm & 0xFFFF) << 7) | rt
}

/// `iohl rt, imm` -- OR the immediate into the lower halfword.
fn iohl(rt: u32, imm: u32) -> u32 {
    0x0C1 << 23 | ((imm & 0xFFFF) << 7) | rt
}

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// Writes `word` to `MFC_Cmd` with a valid 16-byte transfer staged.
fn issue(word: u32) -> (SpuExecutionUnit, ExecutionStepResult, Vec<Effect>) {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let program = [
        ilhu(11, word >> 16),
        iohl(11, word & 0xFFFF),
        wrch(MFC_CMD, 11),
    ];
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    s.channels.mfc_lsa = 0x1000;
    s.channels.mfc_eal = 0x2000;
    s.channels.mfc_size = 16;
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (unit, result, effects)
}

/// The error of the one invalid command `effects` queues, carrying `word`.
fn queued_error(word: u32, effects: &[Effect]) -> MfcCommandError {
    match effects {
        [Effect::MfcInvalidCommand { command, .. }] => {
            assert_eq!(command.word, word, "the record keeps the word");
            command.error
        }
        other => panic!("expected one invalid command, got {other:?}"),
    }
}

fn assert_queued(word: u32, want: MfcCommandError) {
    let (unit, result, effects) = issue(word);
    assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
    assert_eq!(queued_error(word, &effects), want);
    assert_eq!(
        unit.state().channels.cmd_queue_free,
        MFC_SPU_QUEUE_DEPTH - 1,
        "the command takes a slot"
    );
}

/// [CBEA p:57 s:7.2 Table 7-6] any reserved bit in the opcode is an invalid MFC command opcode.
#[test]
fn a_reserved_opcode_queues_an_invalid_command_though_its_low_byte_names_a_put() {
    assert_queued(0x8020, MfcCommandError::ReservedOpcode(0x8020));
}

/// [CBEA p:53 s:7.1] an opcode neither defined nor reserved is illegal.
#[test]
fn an_illegal_opcode_queues_an_invalid_command() {
    assert_queued(0x0027, MfcCommandError::IllegalOpcode(0x0027));
    assert_queued(0x0120, MfcCommandError::IllegalOpcode(0x0120));
}

/// [CBEA p:57 s:7.2 Table 7-6] an `s` command issued to the SPU command queue is an invalid command for that queue.
#[test]
fn an_s_command_queues_an_invalid_command() {
    for opcode in [0x28, 0x2A, 0x29, 0x48, 0x4A, 0x49] {
        assert_queued(opcode, MfcCommandError::ProxyOnlyCommand(opcode));
    }
}

#[test]
fn class_ids_do_not_change_the_opcode_class() {
    let word = 0x0302_0000 | 0x0048;
    let (_, _, effects) = issue(word);
    assert_eq!(
        queued_error(word, &effects),
        MfcCommandError::ProxyOnlyCommand(0x48)
    );
}

/// Opcode 0x89 is sdcrz, which the SPU queue accepts.
#[test]
fn a_defined_command_the_model_does_not_run_faults() {
    let (_, result, effects) = issue(0x0089);
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_UNSUPPORTED_MFC_CMD | 0x89))
    );
    assert!(effects.is_empty(), "{effects:?}");
}
