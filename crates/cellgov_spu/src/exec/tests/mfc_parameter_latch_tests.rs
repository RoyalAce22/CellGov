//! What the MFC command parameter channels hold after a command: the
//! last-written values, except `MFC_EAH`, which returns to 0.

use crate::SpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GETLLAR, MFC_PUT, MFC_SPU_QUEUE_DEPTH};
use cellgov_time::Budget;

const LSA: u32 = 0x1000;
const EAL: u32 = 0x2000;
const SIZE: u32 = 16;
const TAG: u32 = 3;

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> u32 {
    0x081 << 23 | (imm << 7) | rt
}

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// A unit that writes `cmd` to `MFC_Cmd` twice, with its parameters
/// staged once and `MFC_EAH` at `eah`.
fn unit_issuing_twice(cmd: u32, eah: u32) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(5));
    let program = [il(11, cmd), wrch(MFC_CMD, 11), wrch(MFC_CMD, 11)];
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    let c = &mut s.channels;
    c.mfc_lsa = LSA;
    c.mfc_eah = eah;
    c.mfc_eal = EAL;
    c.mfc_size = SIZE;
    c.mfc_tag_id = TAG;
    unit
}

fn step(unit: &mut SpuExecutionUnit) -> (YieldReason, Vec<Effect>) {
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result.yield_reason, effects)
}

/// The main-storage start of the one transfer `effects` enqueues.
fn put_destination(effects: &[Effect]) -> u64 {
    match effects {
        [Effect::DmaEnqueue { request, .. }] => request.destination().start().raw(),
        other => panic!("expected one enqueue, got {other:?}"),
    }
}

/// [CBEA p:121 s:9.2] after a command is queued its parameter values become invalid; the architecture does not say what the channels then hold.
/// [CBEA p:52 s:7] when EAH is not specified on a command, hardware must set EAH to '0'.
#[test]
fn a_command_leaves_the_required_parameters_and_clears_the_high_word() {
    let mut unit = unit_issuing_twice(MFC_PUT, 1);
    assert_eq!(step(&mut unit).0, YieldReason::DmaSubmitted);
    let c = &unit.state().channels;
    assert_eq!(
        (c.mfc_lsa, c.mfc_eal, c.mfc_size, c.mfc_tag_id),
        (LSA, EAL, SIZE, TAG),
        "the required parameters keep their last-written values"
    );
    assert_eq!(c.mfc_eah, 0);
}

/// A second command that rewrites nothing reuses the first one's
/// required parameters, and names the high word 0.
#[test]
fn a_second_command_reuses_the_parameters_it_did_not_rewrite() {
    let eah = 0x0000_0001;
    let mut unit = unit_issuing_twice(MFC_PUT, eah);
    let (_, first) = step(&mut unit);
    assert_eq!(
        put_destination(&first),
        u64::from(eah) << 32 | u64::from(EAL),
        "the first put used the high word it was given"
    );
    let (_, second) = step(&mut unit);
    assert_eq!(
        put_destination(&second),
        u64::from(EAL),
        "the second did not write MFC_EAH, so its high word is 0"
    );
}

/// A write that stalls on a full queue used nothing, so the high word
/// waits for the write that runs.
#[test]
fn a_command_that_stalls_on_a_full_queue_keeps_the_high_word() {
    let mut unit = unit_issuing_twice(MFC_PUT, 1);
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem).with_dma_queue_occupancy(MFC_SPU_QUEUE_DEPTH);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::ChannelStall);
    assert_eq!(unit.state().channels.mfc_eah, 1);
}

/// A refused getllar's record names the line it moves, in its parameters
/// as in its error.
#[test]
fn a_refused_getllar_records_its_line() {
    let mut unit = unit_issuing_twice(MFC_GETLLAR, 1);
    unit.state_mut().channels.mfc_eal = 0x9_002C;
    let (_, effects) = step(&mut unit);
    match effects.as_slice() {
        [Effect::MfcInvalidCommand { command, .. }] => {
            assert_eq!(
                command.params.ea(),
                0x1_0009_0000,
                "built before the high word returned to 0"
            );
            assert_eq!(
                command.error,
                cellgov_dma::MfcCommandError::DataStorage { ea: 0x1_0009_0000 }
            );
        }
        other => panic!("expected one invalid command, got {other:?}"),
    }
}

/// A command the model does not run was never enqueued, so it uses no
/// parameter and the high word stays.
#[test]
fn a_command_the_model_does_not_run_keeps_the_high_word() {
    let mut unit = unit_issuing_twice(cellgov_ps3_abi::hw::spu::MFC_PUTB, 1);
    assert_eq!(step(&mut unit).0, YieldReason::Fault);
    assert_eq!(unit.state().channels.mfc_eah, 1);
}
