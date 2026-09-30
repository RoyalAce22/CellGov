//! A put or get whose parameters the MFC refuses still retires: it joins
//! the command queue as an invalid command, and the queue suspends when
//! it reaches it.
//!
//! The architecture checks the staged parameters asynchronous to the
//! instruction stream, so the `wrch` retires and the MFC command queue
//! suspends. A staged tag id past 31 sets a reserved bit above the tag
//! field, so it never reaches a transfer.

// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] the identification tag is any value between x'0' and x'1F'.

use crate::SpuExecutionUnit;
use cellgov_dma::MfcCommandError;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_GET, MFC_GETLLAR, MFC_PUT, MFC_TAG_ID};
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

fn run_once(unit: &mut SpuExecutionUnit) -> (cellgov_exec::ExecutionStepResult, Vec<Effect>) {
    let mem = GuestMemory::new(MEM_BYTES);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result, effects)
}

/// The tag of the get the step enqueued, or `None` when it enqueued none.
fn enqueued_tag(effects: &[Effect]) -> Option<u32> {
    effects.iter().find_map(|effect| match effect {
        Effect::DmaEnqueue { request, .. } => request.tag_id().map(|tag| u32::from(tag.raw())),
        _ => None,
    })
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
    let (result, _) = run_once(&mut unit);

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
    let (result, effects) = run_once(&mut unit);

    assert_eq!(
        result.yield_reason,
        YieldReason::DmaSubmitted,
        "31 is inside the range, so the command is enqueued",
    );
    assert_eq!(
        enqueued_tag(&effects),
        Some(LAST_VALID_TAG),
        "and the transfer carries the tag the guest named",
    );
}

/// The error of the invalid command the step queued.
fn invalid_error(effects: &[Effect]) -> Option<MfcCommandError> {
    effects.iter().find_map(|effect| match effect {
        Effect::MfcInvalidCommand { command, .. } => Some(command.error),
        _ => None,
    })
}

/// [CBEA p:57 s:7.2 Table 7-6] a reserved tag bit is an invalid DMA command.
/// [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] A set bit above the tag field suspends MFC command queue processing.
#[test]
fn a_tag_id_past_the_architected_range_queues_an_invalid_command() {
    for tag in [FIRST_INVALID_TAG, 0x100, u32::MAX] {
        let mut unit = unit_getting_with_tag(tag);
        let (result, effects) = run_once(&mut unit);
        assert_eq!(
            result.yield_reason,
            YieldReason::DmaSubmitted,
            "tag 0x{tag:x}"
        );
        assert_eq!(
            invalid_error(&effects),
            Some(MfcCommandError::ReservedTagBits(tag)),
            "the whole staged value is checked, not its low byte"
        );
        assert_eq!(enqueued_tag(&effects), None, "no transfer was queued");
        assert_eq!(unit.status(), UnitStatus::Runnable, "the SPU runs on");
        assert_eq!(unit.state().pc, 16, "the wrch retired");
        assert_eq!(
            unit.state().channels.cmd_queue_free,
            cellgov_ps3_abi::hw::spu::MFC_SPU_QUEUE_DEPTH - 1,
            "the command holds a slot"
        );
    }
}

/// A unit whose channels name a transfer of `size` bytes between `lsa`
/// and `eal` under tag 3, and which issues `command`.
fn unit_issuing(command: u32, lsa: u32, eal: u32, size: u32) -> SpuExecutionUnit {
    let mut unit = unit_running(&[il(11, command), wrch(MFC_CMD, 11)]);
    let s = unit.state_mut();
    s.channels.mfc_lsa = lsa;
    s.channels.mfc_eal = eal;
    s.channels.mfc_size = size;
    s.channels.mfc_tag_id = 3;
    unit
}

/// [CBEA p:57 s:7.2 Table 7-6] a size, local-store alignment or effective-address alignment error queues an invalid command for a put or a get.
#[test]
fn an_alignment_error_queues_an_invalid_command_for_a_put_or_a_get() {
    for command in [MFC_PUT, MFC_GET] {
        for (lsa, eal, size, error) in [
            (0x100, 0x40, 3, MfcCommandError::SizeUnaligned(3)),
            (
                0x102,
                0x42,
                4,
                MfcCommandError::LocalStoreUnaligned {
                    lsa: 0x102,
                    size: 4,
                },
            ),
            (
                0x100,
                0x44,
                16,
                MfcCommandError::AddressLowBitsDiffer {
                    lsa: 0x100,
                    ea: 0x44,
                },
            ),
            (0x100, 0x40, 0x4010, MfcCommandError::SizeTooLarge(0x4010)),
        ] {
            let mut unit = unit_issuing(command, lsa, eal, size);
            let (result, effects) = run_once(&mut unit);
            assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
            assert_eq!(
                invalid_error(&effects),
                Some(error),
                "command 0x{command:x}"
            );
            assert_eq!(unit.status(), UnitStatus::Runnable);
        }
    }
}

/// [CBEA p:57 s:7.2 Table 7-6] footnote 1: the alignment checks do not apply to the atomic commands; [CBEA p:66 s:7.8.1] getllar takes no tag.
#[test]
fn getllar_runs_whatever_the_tag_size_and_alignment_say() {
    let mut unit = unit_issuing(MFC_GETLLAR, 0x103, 0x80, 3);
    unit.state_mut().channels.mfc_tag_id = u32::MAX;
    let (result, effects) = run_once(&mut unit);
    assert_eq!(invalid_error(&effects), None);
    assert_ne!(result.yield_reason, YieldReason::Fault);
}
