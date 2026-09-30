//! A tag id the tag-status word has no bit for refuses its command.
//!
//! The channel write itself stands: the architecture checks the staged
//! parameter asynchronous to the instruction stream, and what it names
//! is a suspended MFC command queue, not a faulted `wrch`. So the
//! refusal sits on the command. The tag-status word has one bit for
//! each of the 32 tag groups, and a value past 31 names none of them.
//! The refusal keeps such a value away from the put path, which expects
//! a valid `MfcTagId`. It also keeps a parked get's tag inside the 32
//! groups.

use crate::fault_codes::FAULT_MFC_TAG_ID_OUT_OF_RANGE;
use crate::SpuExecutionUnit;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GET, MFC_TAG_ID};
use cellgov_time::Budget;

const UNIT: u64 = 7;
const MEM_BYTES: usize = 0x2000;

/// The highest tag id the architecture allows, and the first one past
/// it.
const LAST_VALID_TAG: u32 = 31;
const FIRST_INVALID_TAG: u32 = 32;

/// The transfer the command under test would perform. Its bytes never
/// move: every case stops at the command's own step.
const TRANSFER_BYTES: u32 = 4;
const TRANSFER_EAL: u32 = 0x40;

/// `il rt, imm` -- RI16, so the value reaches the preferred slot.
fn il(rt: u32, imm: u32) -> u32 {
    0x081 << 23 | ((imm & 0xFFFF) << 7) | rt
}

/// `wrch $ch<channel>, rt` -- the channel sits in the RA field.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// A unit whose local store holds `words` from address zero. What
/// follows them is zero, which decodes as `stop`.
fn unit_running(words: &[u32]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(UNIT));
    let s = unit.state_mut();
    for (i, word) in words.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    unit
}

/// A unit that writes `tag` to `MFC_TagID` and stops, naming no command.
fn unit_writing_tag(tag: u32) -> SpuExecutionUnit {
    unit_running(&[il(10, tag), wrch(MFC_TAG_ID, 10)])
}

/// A unit that writes `tag` to `MFC_TagID` and then enqueues a get
/// carrying it, which is the command under test.
fn unit_getting_with_tag(tag: u32) -> SpuExecutionUnit {
    let mut unit = unit_running(&[
        il(10, tag),
        wrch(MFC_TAG_ID, 10),
        il(11, MFC_GET),
        wrch(MFC_CMD, 11),
    ]);
    let s = unit.state_mut();
    s.channels.mfc_size = TRANSFER_BYTES;
    s.channels.mfc_eal = TRANSFER_EAL;
    unit
}

fn run_once(unit: &mut SpuExecutionUnit) -> cellgov_exec::ExecutionStepResult {
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(100), &ctx, &mut effects)
}

/// The premise: the encoding puts the value on the channel, so the
/// cases below differ by the tag id and nothing else.
#[test]
fn the_program_writes_the_tag_id_it_was_built_with() {
    let mut unit = unit_writing_tag(LAST_VALID_TAG);
    let _ = run_once(&mut unit);
    assert_eq!(
        unit.state().channels.mfc_tag_id,
        LAST_VALID_TAG,
        "the write reached the channel",
    );
}

/// The write of an out-of-range tag id is not itself refused.
///
/// The architecture checks the staged parameter asynchronous to the
/// instruction stream, so a program that writes a wide value and names
/// no command runs on. Refusing here would stop a program the hardware
/// keeps running.
#[test]
fn the_channel_write_is_not_where_a_wide_tag_id_is_checked() {
    let mut unit = unit_writing_tag(FIRST_INVALID_TAG);
    let result = run_once(&mut unit);

    assert_ne!(
        result.yield_reason,
        YieldReason::Fault,
        "no command names the tag, so nothing is refused",
    );
    assert_eq!(
        unit.state().channels.mfc_tag_id,
        FIRST_INVALID_TAG,
        "and the value the guest wrote is what the channel holds",
    );
}

/// The last valid tag id issues its command.
///
/// Without this the refusal below could pass against a gate that
/// refused every tag id.
#[test]
fn the_highest_architected_tag_id_issues_its_command() {
    let mut unit = unit_getting_with_tag(LAST_VALID_TAG);
    let result = run_once(&mut unit);

    assert_eq!(
        result.yield_reason,
        YieldReason::DmaSubmitted,
        "31 is inside the range, so the command is enqueued",
    );
    assert_eq!(
        unit.state()
            .channels
            .pending_get
            .map(|(_, _, _, tag)| u32::from(tag.raw())),
        Some(LAST_VALID_TAG),
        "and the transfer carries the tag the guest named",
    );
}

/// One past it refuses the command by name.
#[test]
fn a_tag_id_past_the_architected_range_refuses_its_command() {
    let mut unit = unit_getting_with_tag(FIRST_INVALID_TAG);
    let result = run_once(&mut unit);

    assert_eq!(
        result.yield_reason,
        YieldReason::Fault,
        "a tag id with no bit in the status word cannot be carried",
    );
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_TAG_ID_OUT_OF_RANGE | FIRST_INVALID_TAG
        )),
        "the refusal names itself and the value the guest wrote",
    );
    assert!(
        unit.state().channels.pending_get.is_none(),
        "no transfer was parked with a tag outside the 32 groups",
    );
    assert_eq!(
        unit.state().channels.tag_status,
        u32::MAX,
        "and no tag group was left outstanding",
    );
    assert_eq!(
        unit.status(),
        UnitStatus::Faulted,
        "the unit stops rather than carrying a command it cannot report",
    );
}

/// A staged value whose low byte is a valid tag id is still refused.
///
/// The channel holds 32 bits and a tag id fits in 8. A narrowing that
/// dropped the high bits would read 0x100 as tag 0 and issue the command.
#[test]
fn a_wide_tag_id_whose_low_byte_is_in_range_refuses_its_command() {
    const LOW_BYTE_ZERO: u32 = 0x100;
    let mut unit = unit_getting_with_tag(LOW_BYTE_ZERO);
    let result = run_once(&mut unit);

    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(
            FAULT_MFC_TAG_ID_OUT_OF_RANGE | LOW_BYTE_ZERO
        )),
        "the whole staged value is checked, not its low byte",
    );
    assert!(
        unit.state().channels.pending_get.is_none(),
        "no transfer was parked under tag 0",
    );
}

/// The detail bits cannot reach the class field, whatever the guest
/// wrote.
///
/// `u32::MAX` is the worst case: unmasked it would set every class bit
/// and decode as some other fault entirely.
#[test]
fn a_refused_tag_id_cannot_smear_into_the_fault_class() {
    let mut unit = unit_getting_with_tag(u32::MAX);
    let result = run_once(&mut unit);

    assert_eq!(
        unit.state().channels.mfc_tag_id,
        u32::MAX,
        "the premise: every bit of the staged tag is set",
    );
    let Some(FaultKind::Guest(code)) = result.fault else {
        panic!("expected a guest fault, got {:?}", result.fault);
    };
    assert_eq!(
        code & 0xFFFF_0000,
        FAULT_MFC_TAG_ID_OUT_OF_RANGE,
        "the class the reader decodes is the class that was raised: \
         0x{code:08x}",
    );
}
