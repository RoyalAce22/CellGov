//! A list command queues one transfer per element, under one tag group
//! and one command-queue slot, and stops at a stall-and-notify element
//! until software acknowledges the stall.

use crate::SpuExecutionUnit;
use cellgov_dma::{DmaDirection, DmaRequest, MfcCommandError};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_GETL, MFC_LIST_STALL_NOTIFY, MFC_PUTL, MFC_RD_LIST_STALL_STAT,
    MFC_SPU_QUEUE_DEPTH, MFC_WR_LIST_STALL_ACK,
};
use cellgov_time::Budget;

const TAG: u32 = 5;
const LIST: u32 = 0x200;
const DATA: u32 = 0x400;

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    0x00D << 21 | (u32::from(channel) << 7) | rt
}

/// A unit that runs `program`, with `elements` staged at [`LIST`].
///
/// Each element is a (flagged size, low effective address) pair. The
/// latched list parameters put the data at [`DATA`] under tag [`TAG`].
/// r2 holds `cmd` and r9 holds the tag.
fn unit(cmd: u32, elements: &[(u32, u32)], program: &[u32]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(4));
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    for (i, &(head, eal)) in elements.iter().enumerate() {
        let at = LIST as usize + 8 * i;
        s.ls[at..at + 4].copy_from_slice(&head.to_be_bytes());
        s.ls[at + 4..at + 8].copy_from_slice(&eal.to_be_bytes());
    }
    for (i, byte) in s.ls[DATA as usize..DATA as usize + 0x100]
        .iter_mut()
        .enumerate()
    {
        *byte = i as u8;
    }
    s.set_reg_word_splat(2, cmd);
    s.set_reg_word_splat(9, TAG);
    let c = &mut s.channels;
    c.mfc_lsa = DATA;
    c.mfc_eal = LIST;
    c.mfc_size = 8 * elements.len() as u32;
    c.mfc_tag_id = TAG;
    unit
}

/// Runs one step. The context reports `stall_tags` as the tag groups
/// with a queued stall-and-notify element.
fn step(unit: &mut SpuExecutionUnit, stall_tags: u32) -> (YieldReason, Vec<Effect>) {
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem).with_list_stall_tags(stall_tags);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(1), &ctx, &mut effects);
    (result.yield_reason, effects)
}

/// Each queued request, in order.
fn requests(effects: &[Effect]) -> Vec<DmaRequest> {
    effects
        .iter()
        .map(|e| match e {
            Effect::DmaEnqueue { request, .. } => *request,
            other => panic!("expected an enqueue, got {other:?}"),
        })
        .collect()
}

/// (local-store address, effective address, size) of a get.
fn get_shape(r: &DmaRequest) -> (u64, u64, u64) {
    (
        r.destination().start().raw(),
        r.source().start().raw(),
        r.length(),
    )
}

/// [CBEA p:59 s:7.4] each transfer starts at the next quadword boundary of local store after the last, and one below 16 bytes takes the low four bits of its effective address.
#[test]
fn a_get_list_queues_one_transfer_per_element_at_the_next_quadword() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x20, 0x1000), (4, 0x2008), (0x10, 0x3000)],
        &[wrch(MFC_CMD, 2)],
    );
    let (reason, effects) = step(&mut unit, 0);
    assert_eq!(reason, YieldReason::DmaSubmitted);
    let queued = requests(&effects);
    assert_eq!(
        queued.iter().map(get_shape).collect::<Vec<_>>(),
        [
            (0x400, 0x1000, 0x20),
            (0x428, 0x2008, 4),
            (0x430, 0x3000, 0x10)
        ]
    );
    assert!(queued
        .iter()
        .all(|r| r.direction() == DmaDirection::Get && r.tag_id().map(|t| t.raw()) == Some(5)));
    assert_eq!(
        queued.iter().map(|r| r.holds_slot()).collect::<Vec<_>>(),
        [false, false, true],
        "the list holds one slot"
    );
    assert_eq!(
        unit.state().channels.cmd_queue_free,
        MFC_SPU_QUEUE_DEPTH - 1
    );
    assert!(unit.state().channels.lists.is_empty());
}

#[test]
fn a_put_list_carries_each_element_bytes_from_its_local_store_address() {
    let mut unit = unit(
        MFC_PUTL,
        &[(0x10, 0x1000), (2, 0x2002)],
        &[wrch(MFC_CMD, 2)],
    );
    let (_, effects) = step(&mut unit, 0);
    let payloads: Vec<Vec<u8>> = effects
        .iter()
        .map(|e| match e {
            Effect::DmaEnqueue {
                payload: Some(p), ..
            } => p.clone(),
            other => panic!("expected a put, got {other:?}"),
        })
        .collect();
    assert_eq!(payloads[0], (0u8..0x10).collect::<Vec<_>>());
    assert_eq!(
        payloads[1],
        [0x12, 0x13],
        "at 0x412: the next quadword, low bits of 0x2002"
    );
}

/// [CBEA p:129 s:9.3.7] the MFC reads no element past one with the stall-and-notify flag; the stall occurs once that element completes, sets its group's bit, and an acknowledgment restarts the list.
#[test]
fn a_list_stops_at_a_stall_element_and_resumes_on_acknowledgment() {
    let bit = 1 << TAG;
    let mut unit = unit(
        MFC_GETL,
        &[
            (0x10, 0x1000),
            (0x10 | MFC_LIST_STALL_NOTIFY, 0x1100),
            (0x10, 0x1200),
        ],
        &[
            wrch(MFC_CMD, 2),
            rdch(MFC_RD_LIST_STALL_STAT, 5),
            wrch(MFC_WR_LIST_STALL_ACK, 9),
        ],
    );
    let (_, effects) = step(&mut unit, 0);
    let queued = requests(&effects);
    assert_eq!(queued.len(), 2, "no element past the stall");
    assert!(queued[1].stall_notify() && !queued[1].holds_slot());
    assert_eq!(unit.state().channels.lists.len(), 1);

    // The flagged element is still queued: no stall yet, so the read parks.
    let (reason, _) = step(&mut unit, bit);
    assert_eq!(reason, YieldReason::ChannelStall);
    let c = &unit.state().channels;
    assert_eq!(c.tag_status & bit, 0, "a stalled list holds its tag group");
    assert_eq!(c.cmd_queue_free, MFC_SPU_QUEUE_DEPTH - 1, "and its slot");

    // It has left the queue: the list stalls, and the read takes the bit.
    let (reason, _) = step(&mut unit, 0);
    assert_eq!(reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().reg_word(5), bit);
    assert_eq!(
        unit.state().channels.list_stall_status,
        0,
        "a read clears it"
    );

    // Software rewrites the element the stall held back.
    unit.state_mut().ls[LIST as usize + 20..LIST as usize + 24]
        .copy_from_slice(&0x1300u32.to_be_bytes());
    let (reason, effects) = step(&mut unit, 0);
    assert_eq!(reason, YieldReason::DmaSubmitted);
    let resumed = requests(&effects);
    assert_eq!(
        resumed.iter().map(get_shape).collect::<Vec<_>>(),
        [(0x420, 0x1300, 0x10)]
    );
    assert!(resumed[0].holds_slot());
    assert!(unit.state().channels.lists.is_empty());
}

/// [CBEA p:59 s:7.4] a stall-and-notify flag on the last element is ignored.
#[test]
fn a_stall_flag_on_the_last_element_stalls_nothing() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x10, 0x1000), (0x10 | MFC_LIST_STALL_NOTIFY, 0x1100)],
        &[wrch(MFC_CMD, 2)],
    );
    let queued = requests(&step(&mut unit, 0).1);
    assert!(!queued[1].stall_notify() && queued[1].holds_slot());
    assert!(unit.state().channels.lists.is_empty());
}

/// [CBEA p:116 s:9.1.4] a list size is a multiple of 8 up to 16 KB, and may be 0.
#[test]
fn an_empty_list_holds_a_slot_and_its_tag_and_moves_nothing() {
    let mut unit = unit(MFC_GETL, &[], &[wrch(MFC_CMD, 2)]);
    let queued = requests(&step(&mut unit, 0).1);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].length(), 0);
    assert!(queued[0].holds_slot());
    assert_eq!(queued[0].tag_id().map(|t| t.raw()), Some(5));
}

/// The refusal of the one invalid command in `effects`.
fn refusal(effects: &[Effect]) -> MfcCommandError {
    match effects {
        [Effect::MfcInvalidCommand { command, .. }] => command.error,
        other => panic!("expected one invalid command, got {other:?}"),
    }
}

/// [CBEA p:116 s:9.1.4] an invalid list size raises the DMA alignment interrupt.
#[test]
fn a_list_size_that_is_not_a_multiple_of_8_is_refused() {
    let mut unit = unit(MFC_GETL, &[(0x10, 0x1000)], &[wrch(MFC_CMD, 2)]);
    unit.state_mut().channels.mfc_size = 12;
    assert_eq!(
        refusal(&step(&mut unit, 0).1),
        MfcCommandError::ListSizeUnaligned(12)
    );
}

#[test]
fn a_refused_element_ends_the_list_as_an_invalid_command_in_its_slot() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x10, 0x1000), (3, 0x1100), (0x10, 0x1200)],
        &[wrch(MFC_CMD, 2)],
    );
    let (_, effects) = step(&mut unit, 0);
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::DmaEnqueue { request, .. },
            Effect::MfcInvalidCommand { command, .. },
        ] if !request.holds_slot() && command.error == MfcCommandError::SizeUnaligned(3)
    ));
    assert_eq!(
        unit.state().channels.cmd_queue_free,
        MFC_SPU_QUEUE_DEPTH - 1
    );
    assert!(unit.state().channels.lists.is_empty());
}

/// A read with no list that has a stall still to come never completes.
#[test]
fn a_stall_read_with_no_list_is_refused_and_an_acknowledgment_restarts_nothing() {
    let mut ack = unit(MFC_GETL, &[], &[wrch(MFC_WR_LIST_STALL_ACK, 9)]);
    let (reason, effects) = step(&mut ack, 0);
    assert_eq!(reason, YieldReason::BudgetExhausted);
    assert!(effects.is_empty());
    let mut read = unit(MFC_GETL, &[], &[rdch(MFC_RD_LIST_STALL_STAT, 5)]);
    assert_eq!(step(&mut read, 0).0, YieldReason::Fault);
}

/// [CBEA p:60 s:7.5.3] the LSA must be 16-byte aligned when the first element is 16 bytes or less.
#[test]
fn a_short_first_element_needs_a_quadword_aligned_local_store_address() {
    let mut unaligned = unit(MFC_GETL, &[(8, 0x1008)], &[wrch(MFC_CMD, 2)]);
    unaligned.state_mut().channels.mfc_lsa = 0x408;
    assert_eq!(
        refusal(&step(&mut unaligned, 0).1),
        MfcCommandError::LocalStoreUnaligned {
            lsa: 0x408,
            size: 8
        }
    );
    let mut aligned = unit(MFC_GETL, &[(8, 0x1008)], &[wrch(MFC_CMD, 2)]);
    let queued = requests(&step(&mut aligned, 0).1);
    assert_eq!(get_shape(&queued[0]), (0x408, 0x1008, 8));
}

/// [CBE-Handbook p:531 s:19.4.4.2] a list element transfer cannot cross the 4 GB area of the list's EAH.
#[test]
fn an_element_that_crosses_its_4_gb_area_is_refused_after_the_elements_before_it() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x10, 0x1000), (0x20, 0xFFFF_FFF0)],
        &[wrch(MFC_CMD, 2)],
    );
    let (_, effects) = step(&mut unit, 0);
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::DmaEnqueue { .. },
                Effect::MfcInvalidCommand { command, .. },
            ] if command.error == MfcCommandError::ListElementCrosses4Gb {
                ea: 0xFFFF_FFF0,
                size: 0x20,
            }
        ),
        "{effects:?}"
    );
}

/// [CBEA p:129 s:9.3.7] software skips a list element by setting its transfer size to zero.
#[test]
fn a_zero_size_element_moves_nothing_and_leaves_the_next_transfer_in_place() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x10, 0x1000), (0, 0x2004), (0x10, 0x3000)],
        &[wrch(MFC_CMD, 2)],
    );
    let queued = requests(&step(&mut unit, 0).1);
    assert_eq!(
        queued.iter().map(get_shape).collect::<Vec<_>>(),
        [
            (0x400, 0x1000, 0x10),
            (0x414, 0x2004, 0),
            (0x410, 0x3000, 0x10)
        ]
    );
}

/// [CBEA p:61 s:7.5.3] a list whose transfer overwrites list elements not yet started gives unpredictable results.
/// The model reads a segment's elements when the segment starts, so a
/// get over them changes only elements after a stall.
#[test]
fn a_list_that_gets_over_its_own_elements_queues_the_elements_it_read_first() {
    let mut unit = unit(
        MFC_GETL,
        &[(0x20, 0x1000), (0x10, 0x2000), (0x10, 0x3000)],
        &[wrch(MFC_CMD, 2)],
    );
    unit.state_mut().channels.mfc_lsa = LIST;
    let queued = requests(&step(&mut unit, 0).1);
    assert_eq!(
        queued.iter().map(get_shape).collect::<Vec<_>>(),
        [
            (0x200, 0x1000, 0x20),
            (0x220, 0x2000, 0x10),
            (0x230, 0x3000, 0x10)
        ]
    );
}
