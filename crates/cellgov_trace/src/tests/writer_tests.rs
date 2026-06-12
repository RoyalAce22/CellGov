//! TraceWriter buffering, level filtering, and take-bytes reset behavior.

use super::*;
use crate::hash::StateHash;
use crate::record::{HashCheckpointKind, TracedYieldReason};
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

fn scheduled() -> TraceRecord {
    TraceRecord::UnitScheduled {
        unit: UnitId::new(0),
        granted_budget: Budget::new(1),
        time: GuestTicks::ZERO,
        epoch: Epoch::ZERO,
    }
}

fn commit() -> TraceRecord {
    TraceRecord::CommitApplied {
        unit: UnitId::new(0),
        writes_committed: 1,
        effects_deferred: 0,
        fault_discarded: false,
        epoch_after: Epoch::new(1),
    }
}

fn hash_checkpoint() -> TraceRecord {
    TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::new(42),
    }
}

fn step() -> TraceRecord {
    TraceRecord::StepCompleted {
        unit: UnitId::new(0),
        yield_reason: TracedYieldReason::BudgetExhausted,
        consumed_cost: InstructionCost::ONE,
        time_after: GuestTicks::new(1),
    }
}

#[test]
fn empty_writer_is_zero_records_zero_bytes() {
    let w = TraceWriter::new();
    assert_eq!(w.record_count(), 0);
    assert_eq!(w.byte_len(), 0);
    assert!(w.bytes().is_empty());
}

#[test]
fn writes_increment_count_and_buffer() {
    let mut w = TraceWriter::new();
    assert!(w.record(&scheduled()));
    assert!(w.record(&step()));
    assert!(w.record(&commit()));
    assert_eq!(w.record_count(), 3);
    assert_eq!(w.byte_len(), 85);
}

#[test]
fn level_filter_drops_disabled_records() {
    let mut w = TraceWriter::with_levels(&[TraceLevel::Commits, TraceLevel::Hashes]);
    assert!(!w.record(&scheduled()));
    assert!(!w.record(&step()));
    assert!(w.record(&commit()));
    assert!(w.record(&hash_checkpoint()));
    assert_eq!(w.record_count(), 2);
}

#[test]
fn empty_filter_drops_everything() {
    let mut w = TraceWriter::with_levels(&[]);
    assert!(!w.record(&scheduled()));
    assert!(!w.record(&commit()));
    assert_eq!(w.record_count(), 0);
    assert_eq!(w.byte_len(), 0);
}

#[test]
fn take_bytes_clears_writer() {
    let mut w = TraceWriter::new();
    w.record(&scheduled());
    let b = w.take_bytes();
    assert!(!b.is_empty());
    assert_eq!(w.byte_len(), 0);
    assert_eq!(w.record_count(), 0);
}
