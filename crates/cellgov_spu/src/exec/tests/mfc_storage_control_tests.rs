//! The SL1 storage control commands queue tag-specific barriers, and
//! sdcrz queues zeros over the data blocks its range touches.

use crate::SpuExecutionUnit;
use cellgov_dma::{DmaDirection, MfcCommandError, MfcOrdering};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, ExecutionContext, ExecutionUnit, RetiredBarrier, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_SDCRF, MFC_SDCRST, MFC_SDCRT, MFC_SDCRTST, MFC_SDCRZ};
use cellgov_sync::{ReservationTable, ReservedLine};
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

/// A unit about to issue `cmd` over `size` bytes at `ea` under `tag`.
fn unit_issuing(cmd: u32, ea: u32, size: u32, tag: u32) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(9));
    let program = [il(11, cmd), wrch(MFC_CMD, 11)];
    let s = unit.state_mut();
    for (i, insn) in program.iter().enumerate() {
        s.ls[i * 4..i * 4 + 4].copy_from_slice(&insn.to_be_bytes());
    }
    let c = &mut s.channels;
    c.mfc_lsa = 0x1000;
    c.mfc_eal = ea;
    c.mfc_size = size;
    c.mfc_tag_id = tag;
    unit
}

/// Runs `unit` to its first yield under per-step tracing, and returns the
/// yield, the effects and the barriers the step retired.
fn run(unit: &mut SpuExecutionUnit) -> (YieldReason, Vec<Effect>, Vec<RetiredBarrier>) {
    let mem = GuestMemory::new(0x4000);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    (result.yield_reason, effects, unit.drain_barriers())
}

/// A queued request's direction, main-storage start and length, tag and
/// ordering.
type Request = (DmaDirection, u64, u64, Option<u8>, MfcOrdering);

/// The one queued request, and its payload.
fn queued(cmd: u32, ea: u32, size: u32) -> (Request, Vec<u8>) {
    let (reason, effects, _) = run(&mut unit_issuing(cmd, ea, size, TAG));
    assert_eq!(reason, YieldReason::DmaSubmitted, "0x{cmd:02x}");
    match effects.as_slice() {
        [Effect::DmaEnqueue {
            request,
            payload: Some(payload),
        }] if request.source().start().raw() == 0 => (
            (
                request.direction(),
                request.destination().start().raw(),
                request.length(),
                request.tag_id().map(|t| t.raw()),
                request.ordering(),
            ),
            payload.clone(),
        ),
        other => panic!("0x{cmd:02x}: expected one enqueue, got {other:?}"),
    }
}

/// [CBEA p:62 s:7.7] each storage control command creates a tag-specific barrier.
/// [CBE-Handbook p:151 s:6.2.2.4 Table 6-1] the CBE runs sdcrt and sdcrtst as nops.
#[test]
fn each_command_but_sdcrz_queues_a_tag_barrier_of_no_bytes() {
    for cmd in [MFC_SDCRT, MFC_SDCRTST, MFC_SDCRST, MFC_SDCRF] {
        let ((direction, _, length, tag, ordering), payload) = queued(cmd, 0x2000, 0x80);
        assert_eq!(
            (direction, length, tag, ordering),
            (
                DmaDirection::Put,
                0,
                Some(TAG as u8),
                MfcOrdering::TagBarrier
            ),
            "0x{cmd:02x}"
        );
        assert!(payload.is_empty(), "0x{cmd:02x}");
    }
}

/// [CBEA p:64 s:7.7.3] sdcrz zeroes every byte of each data block that holds an addressed byte.
/// [CBE-Handbook p:149 s:6.2.2] the CBE's blocks are the atomic cache's 128-byte lines.
#[test]
fn sdcrz_queues_zeros_over_every_block_its_range_touches() {
    let (request, payload) = queued(MFC_SDCRZ, 0x2010, 0x80);
    assert_eq!(
        request,
        (
            DmaDirection::Put,
            0x2000,
            0x100,
            Some(TAG as u8),
            MfcOrdering::TagBarrier
        )
    );
    assert_eq!(payload, [0; 0x100]);

    let (request, _) = queued(MFC_SDCRZ, 0x2000, 0x80);
    assert_eq!((request.1, request.2), (0x2000, 0x80), "one aligned block");
}

#[test]
fn sdcrz_of_no_bytes_zeroes_nothing() {
    let (request, payload) = queued(MFC_SDCRZ, 0x2010, 0);
    assert_eq!(request.2, 0);
    assert!(payload.is_empty());
}

/// [CBEA p:65 s:7.8] a store by the issuer to its reserved line clears its reservation.
#[test]
fn sdcrz_clears_a_reservation_on_a_block_it_zeroes() {
    for (line, kept) in [(0x2080, false), (0x2100, true)] {
        let mut unit = unit_issuing(MFC_SDCRZ, 0x2010, 0x80, TAG);
        unit.state_mut().reservation = Some(ReservedLine::containing(line));
        let mut table = ReservationTable::new();
        table.insert_or_replace(UnitId::new(9), ReservedLine::containing(line));
        let mem = GuestMemory::new(0x4000);
        let ctx = ExecutionContext::new(&mem).with_reservations(&table);
        let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
        assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
        assert_eq!(unit.state().reservation.is_some(), kept, "line 0x{line:x}");
    }
}

/// [CBEA p:57 s:7.2 Table 7-6] a transfer size above 16 KB is an alignment error.
/// The touch forms check only their tag.
#[test]
fn a_storage_control_command_over_16_kb_is_refused_unless_it_is_a_touch() {
    for cmd in [MFC_SDCRZ, MFC_SDCRST, MFC_SDCRF] {
        let (_, effects, _) = run(&mut unit_issuing(cmd, 0x2000, 0x4080, TAG));
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::MfcInvalidCommand { command, .. }]
                    if command.error == MfcCommandError::SizeTooLarge(0x4080)
            ),
            "0x{cmd:02x}: {effects:?}"
        );
    }
    for cmd in [MFC_SDCRT, MFC_SDCRTST] {
        let (reason, _, _) = run(&mut unit_issuing(cmd, 0x2000, 0x4080, TAG));
        assert_eq!(reason, YieldReason::DmaSubmitted, "0x{cmd:02x}");
    }
}

#[test]
fn sdcrz_whose_blocks_run_past_2_64_is_refused_as_a_segment_fault() {
    // The range itself runs past 2^64, or only its last block does.
    for size in [0x80, 0x60] {
        let mut unit = unit_issuing(MFC_SDCRZ, 0xFFFF_FF90, size, TAG);
        unit.state_mut().channels.mfc_eah = u32::MAX;
        let (_, effects, _) = run(&mut unit);
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::MfcInvalidCommand { command, .. }]
                    if command.word == MFC_SDCRZ
                        && command.error == MfcCommandError::DataSegment { ea: 0xFFFF_FFFF_FFFF_FF90 }
            ),
            "size 0x{size:x}: {effects:?}"
        );
    }
}

#[test]
fn a_storage_control_command_with_a_reserved_tag_is_refused() {
    for cmd in [MFC_SDCRT, MFC_SDCRTST, MFC_SDCRZ, MFC_SDCRST, MFC_SDCRF] {
        let (_, effects, _) = run(&mut unit_issuing(cmd, 0x2000, 0x80, 0x40));
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
fn a_traced_step_records_each_storage_control_command_as_a_tag_barrier() {
    for cmd in [MFC_SDCRT, MFC_SDCRTST, MFC_SDCRZ, MFC_SDCRST, MFC_SDCRF] {
        let (_, _, barriers) = run(&mut unit_issuing(cmd, 0x2000, 0x80, TAG));
        assert_eq!(
            barriers,
            [RetiredBarrier {
                pc: 4,
                kind: BarrierKind::MfcTagBarrier
            }],
            "0x{cmd:02x}"
        );
    }
}
