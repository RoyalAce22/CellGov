//! `TraceRecord::tag` / `encoded_len` are the single statement of the
//! wire size of every variant.

use std::collections::BTreeSet;

use super::codec::*;
use super::error::*;
use super::reasons::*;
use super::trace_record::*;
use crate::hash::StateHash;
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

/// One instance of every variant.
fn one_of_each() -> Vec<TraceRecord> {
    vec![
        TraceRecord::RunIdentity {
            format_version: TRACE_FORMAT_VERSION,
            firmware: 31,
            game: 32,
            overrides: 33,
        },
        TraceRecord::UnitScheduled {
            unit: UnitId::new(1),
            granted_budget: Budget::new(2),
            time: GuestTicks::new(3),
            epoch: Epoch::new(4),
        },
        TraceRecord::StepCompleted {
            unit: UnitId::new(1),
            yield_reason: TracedYieldReason::Syscall,
            consumed_cost: InstructionCost::new(5),
            time_after: GuestTicks::new(6),
        },
        TraceRecord::CommitApplied {
            unit: UnitId::new(1),
            writes_committed: 7,
            effects_deferred: 8,
            fault_discarded: true,
            epoch_after: Epoch::new(9),
        },
        TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::SyncState,
            hash: StateHash::new(10),
        },
        TraceRecord::EffectEmitted {
            unit: UnitId::new(1),
            sequence: 11,
            kind: TracedEffectKind::RsxFlipRequest,
        },
        TraceRecord::UnitBlocked {
            unit: UnitId::new(1),
            reason: TracedBlockReason::DmaWait,
        },
        TraceRecord::UnitWoken {
            unit: UnitId::new(1),
            reason: TracedWakeReason::Timer,
        },
        TraceRecord::PpuStateHash {
            step: 12,
            pc: 13,
            hash: StateHash::new(14),
        },
        TraceRecord::PpuStateFull {
            step: 15,
            pc: 16,
            gpr: [17; 32],
            lr: 18,
            ctr: 19,
            xer: 20,
            cr: 21,
            reservation_line: Some(22),
        },
        TraceRecord::HostInvariantBreak {
            reason: TracedInvariantBreakReason::Unspecified,
        },
        TraceRecord::SyscallEntered {
            unit: UnitId::new(1),
            num: 23,
            args: [24; 8],
            disposition: TracedSyscallDisposition::TimerFastPath,
        },
        TraceRecord::ReservedRegionRead {
            unit: UnitId::new(1),
            step: 25,
            addr: 26,
            len: 27,
            hits: 28,
        },
        TraceRecord::SyscallReturned {
            unit: UnitId::new(1),
            code: 29,
            time: GuestTicks::new(30),
        },
        TraceRecord::HostWrite {
            writer: HostWriter::SharedViewFanout,
            space: 33,
            addr: 34,
            len: 35,
            reservations_cleared: 36,
        },
        TraceRecord::StateHashScheme {
            ppu: 37,
            checkpoint: 38,
        },
    ]
}

#[test]
fn every_variant_encodes_to_its_declared_len_behind_its_tag() {
    for record in one_of_each() {
        let mut buf = Vec::new();
        record.encode(&mut buf);
        assert_eq!(buf[0], record.tag(), "{record:?}");
        assert_eq!(
            Some(buf.len()),
            TraceRecord::encoded_len(record.tag()),
            "{record:?}"
        );
    }
}

#[test]
fn every_known_tag_has_a_sample_and_the_next_tag_is_free() {
    let sampled: BTreeSet<u8> = one_of_each().iter().map(TraceRecord::tag).collect();
    let declared: BTreeSet<u8> = (0..=u8::MAX)
        .filter(|&t| TraceRecord::encoded_len(t).is_some())
        .collect();
    assert_eq!(sampled, declared, "a variant is missing from one_of_each");
    assert_eq!(
        declared.iter().copied().collect::<Vec<_>>(),
        (0..=TAG_STATE_HASH_SCHEME).collect::<Vec<_>>(),
        "tags are dense and append-only"
    );
    assert_eq!(TraceRecord::encoded_len(TAG_STATE_HASH_SCHEME + 1), None);
}

#[test]
fn any_prefix_shorter_than_the_declared_len_is_truncated() {
    for record in one_of_each() {
        let mut buf = Vec::new();
        record.encode(&mut buf);
        for cut in 1..buf.len() {
            assert_eq!(
                TraceRecord::decode(&buf[..cut]),
                Err(DecodeError::Truncated),
                "{record:?} cut to {cut} bytes"
            );
        }
        let (decoded, used) = TraceRecord::decode(&buf).expect("full record decodes");
        assert_eq!(decoded, record);
        assert_eq!(used, buf.len());
    }
}
