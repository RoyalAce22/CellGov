//! The fence and barrier forms, sndsig and the ordering commands queue
//! what their opcode names, and a traced step records each one's barrier.

use crate::SpuExecutionUnit;
use cellgov_dma::{DmaDirection, MfcCommandError, MfcOrdering};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, ExecutionContext, ExecutionUnit, RetiredBarrier, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_BARRIER, MFC_CMD, MFC_EIEIO, MFC_GET, MFC_GETB, MFC_GETF, MFC_GETLB, MFC_PUT, MFC_PUTB,
    MFC_PUTF, MFC_PUTLF, MFC_PUTQLLUC, MFC_SNDSIG, MFC_SNDSIGB, MFC_SNDSIGF, MFC_SYNC,
};
use cellgov_time::Budget;

const TAG: u32 = 3;

/// `il $rt, imm`.
fn il(rt: u32, imm: u32) -> u32 {
    0x081 << 23 | (imm << 7) | rt
}

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    0x10D << 21 | (u32::from(channel) << 7) | rt
}

/// Issues `cmd` with a `size`-byte transfer staged under `tag`, and
/// returns the step's yield and effects.
fn issue(cmd: u32, size: u32, tag: u32) -> (YieldReason, Vec<Effect>) {
    let (reason, effects, _) = issue_traced(cmd, size, tag, false);
    (reason, effects)
}

/// As [`issue`], with per-step tracing set by `per_step`, and also
/// returns the barriers the step retired.
fn issue_traced(
    cmd: u32,
    size: u32,
    tag: u32,
    per_step: bool,
) -> (YieldReason, Vec<Effect>, Vec<RetiredBarrier>) {
    let mut unit = SpuExecutionUnit::new(UnitId::new(9));
    let program = [il(11, cmd), wrch(MFC_CMD, 11)];
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    let c = &mut s.channels;
    c.mfc_lsa = 0x1000;
    c.mfc_eal = 0x2000;
    c.mfc_size = size;
    c.mfc_tag_id = tag;
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(per_step);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result.yield_reason, effects, unit.drain_barriers())
}

/// The one queued transfer's direction, length, tag and ordering.
fn queued(cmd: u32, size: u32) -> (DmaDirection, u64, Option<u8>, MfcOrdering) {
    let (reason, effects) = issue(cmd, size, TAG);
    assert_eq!(reason, YieldReason::DmaSubmitted, "0x{cmd:02x}");
    match effects.as_slice() {
        [Effect::DmaEnqueue { request, .. }] => (
            request.direction(),
            request.length(),
            request.tag_id().map(|t| t.raw()),
            request.ordering(),
        ),
        other => panic!("0x{cmd:02x}: expected one enqueue, got {other:?}"),
    }
}

/// [CBEA p:306 s:Appendix D Table D-2] putf x'0022', putb x'0021'.
/// [CBEA p:307 s:Appendix D Table D-2] getf x'0042', getb x'0041'.
#[test]
fn each_put_and_get_form_queues_its_ordering() {
    use DmaDirection::{Get, Put};
    let tag = Some(TAG as u8);
    for (cmd, direction, ordering) in [
        (MFC_PUT, Put, MfcOrdering::None),
        (MFC_PUTF, Put, MfcOrdering::Fence),
        (MFC_PUTB, Put, MfcOrdering::TagBarrier),
        (MFC_GET, Get, MfcOrdering::None),
        (MFC_GETF, Get, MfcOrdering::Fence),
        (MFC_GETB, Get, MfcOrdering::TagBarrier),
    ] {
        assert_eq!(
            queued(cmd, 16),
            (direction, 16, tag, ordering),
            "0x{cmd:02x}"
        );
    }
}

/// [CBEA p:308 s:Appendix D Table D-4] sndsig is a 4-byte put, with fence and barrier forms.
#[test]
fn each_sndsig_form_queues_a_four_byte_put() {
    let tag = Some(TAG as u8);
    for (cmd, ordering) in [
        (MFC_SNDSIG, MfcOrdering::None),
        (MFC_SNDSIGF, MfcOrdering::Fence),
        (MFC_SNDSIGB, MfcOrdering::TagBarrier),
    ] {
        assert_eq!(
            queued(cmd, 4),
            (DmaDirection::Put, 4, tag, ordering),
            "0x{cmd:02x}"
        );
    }
}

/// [CBEA p:57 s:7.2 Table 7-6] a sndsig transfer size other than 4 bytes is an alignment error.
#[test]
fn a_sndsig_of_any_other_size_is_refused() {
    let (_, effects) = issue(MFC_SNDSIG, 16, TAG);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::MfcInvalidCommand { command, .. }]
                if command.error == MfcCommandError::SendSignalSize(16)
        ),
        "{effects:?}"
    );
}

/// [CBEA p:71 s:7.9.1] mfcsync creates a tag-specific barrier, and [CBEA p:72 s:7.9.2] so does mfceieio.
/// [CBEA p:72 s:7.9.3] the barrier command orders the whole queue, and its tag says when it is complete.
#[test]
fn each_ordering_command_queues_no_bytes() {
    let tag = Some(TAG as u8);
    assert_eq!(
        queued(MFC_SYNC, 16),
        (DmaDirection::Put, 0, tag, MfcOrdering::TagBarrier)
    );
    assert_eq!(
        queued(MFC_EIEIO, 16),
        (DmaDirection::Put, 0, tag, MfcOrdering::TagBarrier)
    );
    assert_eq!(
        queued(MFC_BARRIER, 16),
        (DmaDirection::Put, 0, tag, MfcOrdering::QueueBarrier)
    );
}

/// [CBEA p:57 s:7.2 Table 7-6] a reserved tag bit is a command error, and footnote 1 exempts the synchronization commands from the alignment checks alone.
#[test]
fn an_ordering_command_with_a_reserved_tag_is_refused() {
    for cmd in [MFC_SYNC, MFC_EIEIO, MFC_BARRIER] {
        let (_, effects) = issue(cmd, 16, 0x40);
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::MfcInvalidCommand { command, .. }]
                    if command.error == MfcCommandError::ReservedTagBits(0x40)
            ),
            "0x{cmd:02x}: {effects:?}"
        );
    }
}

#[test]
fn a_traced_step_records_the_barrier_each_ordered_command_queues() {
    use BarrierKind::{MfcBarrier, MfcEieio, MfcFence, MfcSync, MfcTagBarrier};
    for (cmd, size, kind) in [
        (MFC_PUTF, 16, MfcFence),
        (MFC_GETF, 16, MfcFence),
        (MFC_SNDSIGF, 4, MfcFence),
        (MFC_PUTB, 16, MfcTagBarrier),
        (MFC_GETB, 16, MfcTagBarrier),
        (MFC_SNDSIGB, 4, MfcTagBarrier),
        (MFC_SYNC, 16, MfcSync),
        (MFC_EIEIO, 16, MfcEieio),
        (MFC_BARRIER, 16, MfcBarrier),
        (MFC_PUTLF, 16, MfcFence),
        (MFC_GETLB, 16, MfcTagBarrier),
    ] {
        let (_, _, barriers) = issue_traced(cmd, size, TAG, true);
        assert_eq!(barriers, [RetiredBarrier { pc: 4, kind }], "0x{cmd:02x}");
        let (_, _, untraced) = issue_traced(cmd, size, TAG, false);
        assert!(untraced.is_empty(), "0x{cmd:02x}");
    }
}

#[test]
fn an_unordered_or_refused_command_records_no_barrier() {
    for (cmd, size, tag) in [
        (MFC_PUT, 16, TAG),
        (MFC_GET, 16, TAG),
        (MFC_SNDSIG, 4, TAG),
        (MFC_SYNC, 16, 0x40),
        (MFC_PUTF, 3, TAG),
    ] {
        let (_, _, barriers) = issue_traced(cmd, size, tag, true);
        assert!(barriers.is_empty(), "0x{cmd:02x} tag {tag}: {barriers:?}");
    }
}

/// [CBEA p:62 s:7.6.5] the result modifier is a cache hint that the CBE does not implement, so each r form runs as its plain form.
#[test]
fn each_result_hint_form_queues_what_its_plain_form_queues() {
    use cellgov_ps3_abi::hw::spu::{
        MFC_PUTL, MFC_PUTLB, MFC_PUTR, MFC_PUTRB, MFC_PUTRF, MFC_PUTRL, MFC_PUTRLB, MFC_PUTRLF,
    };
    for (hint, plain) in [
        (MFC_PUTR, MFC_PUT),
        (MFC_PUTRF, MFC_PUTF),
        (MFC_PUTRB, MFC_PUTB),
        (MFC_PUTRL, MFC_PUTL),
        (MFC_PUTRLF, MFC_PUTLF),
        (MFC_PUTRLB, MFC_PUTLB),
    ] {
        let (hint_reason, hint_effects, hint_barriers) = issue_traced(hint, 16, TAG, true);
        let (plain_reason, plain_effects, plain_barriers) = issue_traced(plain, 16, TAG, true);
        assert_eq!(hint_reason, YieldReason::DmaSubmitted, "0x{hint:02x}");
        assert_eq!(hint_reason, plain_reason, "0x{hint:02x}");
        for (effects, word) in [(&hint_effects, hint), (&plain_effects, plain)] {
            let words = command_words(effects);
            assert!(!words.is_empty(), "0x{word:02x} queues a transfer");
            assert!(
                words.iter().all(|w| *w == Some(word)),
                "0x{word:02x}: {words:?}"
            );
        }
        assert_eq!(
            with_word_zeroed(hint_effects),
            with_word_zeroed(plain_effects),
            "0x{hint:02x}"
        );
        assert_eq!(hint_barriers, plain_barriers, "0x{hint:02x}");
    }
}

/// The command word each queued transfer of `effects` carries.
fn command_words(effects: &[Effect]) -> Vec<Option<u32>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DmaEnqueue { request, .. } => Some(request.command_word()),
            _ => None,
        })
        .collect()
}

/// `effects` with each queued transfer's command word set to 0: the form
/// a refusal reports is the one difference a result hint makes.
fn with_word_zeroed(effects: Vec<Effect>) -> Vec<Effect> {
    effects
        .into_iter()
        .map(|effect| match effect {
            Effect::DmaEnqueue { request, payload } => Effect::DmaEnqueue {
                request: request.with_command_word(0),
                payload,
            },
            other => other,
        })
        .collect()
}

/// [CBEA p:68 s:7.8.4] putqlluc is a queued put of one cache line with an implied tag-specific fence.
#[test]
fn putqlluc_queues_a_fenced_put_of_the_line_from_local_store() {
    let (reason, effects, barriers) = issue_traced(MFC_PUTQLLUC, 0, TAG, true);
    assert_eq!(reason, YieldReason::DmaSubmitted);
    let [Effect::DmaEnqueue {
        request,
        payload: None,
    }] = effects.as_slice()
    else {
        panic!("one queued put: {effects:?}");
    };
    assert_eq!(request.direction(), DmaDirection::Put);
    assert!(request.local_store_source());
    assert_eq!(request.ordering(), MfcOrdering::Fence);
    assert_eq!(request.command_word(), Some(MFC_PUTQLLUC));
    assert_eq!(request.tag_id().map(|t| t.raw()), Some(TAG as u8));
    assert_eq!(
        (
            request.source().start().raw(),
            request.destination().start().raw(),
            request.length()
        ),
        (0x1000, 0x2000, 128),
        "the line, whatever MFC_Size holds"
    );
    assert_eq!(barriers.len(), 1);
    assert_eq!(barriers[0].kind, BarrierKind::MfcFence);
}
