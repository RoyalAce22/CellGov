//! A command word carries an opcode and two class ids, and only the
//! opcode names the operation.
//!
//! [`MfcCmd`] holds the field layout of the word the guest writes to
//! `MFC_Cmd`. A model that matches the word whole refuses every command
//! that carries a class id, which is what the channel exists to let a
//! guest set.

use crate::{SpuExecutionUnit, FAULT_LS_OUT_OF_RANGE, FAULT_UNSUPPORTED_MFC_CMD};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MfcCmd, MFC_CMD, MFC_PUT, MFC_PUTLLC, SPU_LS_SIZE};
use cellgov_sync::{ReservationTable, ReservedLine};
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// The transfer every case below performs.
const TRANSFER_BYTES: u32 = 4;
const TRANSFER_EAL: u32 = 0x40;

/// A transfer class id the guest sets, in bits 0:7 of the word.
const TCLASS: u32 = 0x03;
/// A replacement class id the guest sets, in bits 8:15.
const RCLASS: u32 = 0x02;

/// `ilhu rt, imm` -- the immediate lands in the upper halfword.
fn ilhu(rt: u32, imm: u32) -> u32 {
    0x082 << 23 | ((imm & 0xFFFF) << 7) | rt
}

/// `iohl rt, imm` -- OR the immediate into the lower halfword.
fn iohl(rt: u32, imm: u32) -> u32 {
    0x0C1 << 23 | ((imm & 0xFFFF) << 7) | rt
}

/// `wrch $ch<channel>, rt` -- the channel sits in the RA field.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// A unit whose local store holds a program that writes `word` to
/// `MFC_Cmd`, with the transfer parameters the cases share staged.
fn unit_issuing(word: u32) -> SpuExecutionUnit {
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
    s.channels.mfc_size = TRANSFER_BYTES;
    s.channels.mfc_eal = TRANSFER_EAL;
    unit
}

fn run_once(unit: &mut SpuExecutionUnit) -> (cellgov_exec::ExecutionStepResult, Vec<Effect>) {
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result, effects)
}

/// Like [`run_once`], with the committed table naming this unit as
/// holding `line`.
///
/// A conditional store succeeds only where the unit's own reservation
/// register and the committed table agree. The entry check drops the
/// register when the table does not name the unit. A case that stages
/// the register alone therefore never reaches the store.
fn run_once_holding(
    unit: &mut SpuExecutionUnit,
    line: ReservedLine,
) -> (cellgov_exec::ExecutionStepResult, Vec<Effect>) {
    let mem = GuestMemory::new(MEM_BYTES);
    let mut table = ReservationTable::new();
    table.insert_or_replace(UnitId::new(UNIT), line);
    let ctx = ExecutionContext::new(&mem).with_reservations(&table);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result, effects)
}

/// Without this premise a case could pass because the word degenerates
/// to the bare opcode it carries.
#[test]
fn the_program_builds_the_command_word_it_was_given() {
    let word = TCLASS << 24 | RCLASS << 16 | MFC_PUT;
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let program = [ilhu(11, word >> 16), iohl(11, word & 0xFFFF)];
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    let _ = run_once(&mut unit);

    assert_eq!(
        unit.state().reg_word(11),
        word,
        "the two immediates reach one 32-bit word",
    );
}

/// The model reads past both class ids, so no other case in this file
/// catches a swap of the two. The two constants differ so that a swap
/// shows here.
#[test]
fn each_field_reads_its_own_byte_of_the_word() {
    let raw = TCLASS << 24 | RCLASS << 16 | MFC_PUT;
    let word = MfcCmd::new(raw);

    assert_eq!(word.tclass_id(), TCLASS as u8, "TclassID is bits 0:7");
    assert_eq!(word.rclass_id(), RCLASS as u8, "RclassID is bits 8:15");
    assert_eq!(word.opcode(), MFC_PUT, "the opcode is bits 24:31");
    assert!(
        !word.names_a_reserved_opcode(),
        "and bit 16 is clear in a word that sets neither reserved bit",
    );
    assert_eq!(word.raw(), raw, "the word keeps what it was given");
}

/// The class ids change how fast a command runs, never what it does, so
/// a model with neither to steer reads past them.
#[test]
fn a_put_carrying_class_ids_is_still_a_put() {
    let word = TCLASS << 24 | RCLASS << 16 | MFC_PUT;
    let mut unit = unit_issuing(word);
    let (result, effects) = run_once(&mut unit);

    assert_eq!(
        result.yield_reason,
        YieldReason::DmaSubmitted,
        "the opcode names a put, so the command is enqueued",
    );
    assert!(
        matches!(effects.as_slice(), [Effect::DmaEnqueue { .. }]),
        "and the transfer it enqueued is the one the parameters staged: {effects:?}",
    );
}

/// The bare opcode still enqueues, so the field decode kept the word
/// this model started from.
#[test]
fn a_put_carrying_no_class_id_is_unchanged() {
    let mut unit = unit_issuing(MFC_PUT);
    let (result, _) = run_once(&mut unit);

    assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
}

/// Bit 16 marks the opcode reserved, so such a word is a different
/// command from the one its low byte spells.
#[test]
fn a_reserved_opcode_is_refused_though_its_low_byte_names_a_put() {
    let word = 1 << 15 | MFC_PUT;
    assert!(
        MfcCmd::new(word).names_a_reserved_opcode(),
        "the premise: the word marks its own opcode reserved",
    );
    assert_eq!(
        MfcCmd::new(word).opcode(),
        MFC_PUT,
        "and its low byte really does name a command the model runs",
    );

    let mut unit = unit_issuing(word);
    let (result, effects) = run_once(&mut unit);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    // The detail half is 16 bits, so it carries the reserved byte and
    // the opcode. A word's class ids sit above that half, so the fault
    // omits them.
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_UNSUPPORTED_MFC_CMD | word)),
        "refused by name, carrying the half of the word that refused it",
    );
    assert!(effects.is_empty(), "and nothing was enqueued: {effects:?}");
}

/// The case asserts the whole fault code. Every other way this program
/// can fail -- a mis-encoded `wrch`, a word that reached some other
/// channel -- also ends in a fault. Only the code separates those from
/// this refusal. The detail half carries the opcode; the class ids sit
/// above it.
#[test]
fn an_unmodelled_opcode_is_refused_whatever_its_class_ids() {
    let unmodelled: u32 = 0x21;
    let word = TCLASS << 24 | RCLASS << 16 | unmodelled;
    let mut unit = unit_issuing(word);
    let (result, effects) = run_once(&mut unit);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_UNSUPPORTED_MFC_CMD | unmodelled)),
        "the MFC refusal, carrying the opcode and not the class ids",
    );
    assert!(effects.is_empty(), "and nothing was enqueued: {effects:?}");
}

/// `MFC_LSA` and `MFC_Size` arrive on separate channels and neither
/// write bounds the pair, so the range is the guest's to choose. A
/// direct index of local store here panics the host on a range it
/// cannot hold.
#[test]
fn a_put_reaching_past_local_store_is_refused() {
    let lsa = (SPU_LS_SIZE - 16) as u32;
    let mut unit = unit_issuing(TCLASS << 24 | RCLASS << 16 | MFC_PUT);
    {
        let s = unit.state_mut();
        s.channels.mfc_lsa = lsa;
        s.channels.mfc_size = 64;
    }
    let (result, effects) = run_once(&mut unit);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_LS_OUT_OF_RANGE | (lsa & 0xFFFF))),
        "refused by name, carrying the staged local-store address",
    );
    assert!(effects.is_empty(), "and nothing was enqueued: {effects:?}");
}

/// The same bound on the conditional store, whose line is a fixed 128
/// bytes wherever `MFC_LSA` points.
#[test]
fn a_putllc_reaching_past_local_store_is_refused() {
    let lsa = (SPU_LS_SIZE - 16) as u32;
    let line = ReservedLine::containing(u64::from(TRANSFER_EAL));
    let mut unit = unit_issuing(MFC_PUTLLC);
    {
        let s = unit.state_mut();
        s.channels.mfc_lsa = lsa;
        s.reservation = Some(line);
    }
    let (result, effects) = run_once_holding(&mut unit, line);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_LS_OUT_OF_RANGE | (lsa & 0xFFFF))),
        "refused by name, carrying the staged local-store address",
    );
    assert!(effects.is_empty(), "and nothing was enqueued: {effects:?}");
}
