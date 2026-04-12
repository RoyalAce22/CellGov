use super::*;
use cellgov_effects::Effect;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use std::cell::Cell;

// Local test doubles -- cellgov_testkit depends on cellgov_core,
// so a reverse dev-dependency would create a cycle.

/// Test unit that consumes the full granted budget every step
/// and counts how many steps it has taken.
struct CountingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl CountingUnit {
    fn new(id: UnitId, max: u64) -> Self {
        Self {
            id,
            steps: Cell::new(0),
            max,
        }
    }
}

impl ExecutionUnit for CountingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::TraceMarker {
                marker: n as u32,
                source: self.id,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build(memory_size: usize, budget: u64, max_steps: usize) -> Runtime {
    Runtime::new(
        GuestMemory::new(memory_size),
        Budget::new(budget),
        max_steps,
    )
}

#[test]
fn new_starts_at_zero() {
    let rt = build(16, 5, 100);
    assert_eq!(rt.time(), GuestTicks::ZERO);
    assert_eq!(rt.epoch(), Epoch::ZERO);
    assert_eq!(rt.steps_taken(), 0);
    assert_eq!(rt.max_steps(), 100);
    assert!(rt.registry().is_empty());
    assert_eq!(rt.memory().size(), 16);
}

#[test]
fn step_with_no_units_returns_no_runnable() {
    let mut rt = build(16, 5, 100);
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
    assert_eq!(rt.steps_taken(), 0);
}

#[test]
fn step_runs_a_registered_unit() {
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(0));
    assert_eq!(s.result.consumed_budget, Budget::new(5));
    assert_eq!(s.time_after, GuestTicks::new(5));
    assert_eq!(rt.time(), GuestTicks::new(5));
    assert_eq!(rt.steps_taken(), 1);
}

#[test]
fn time_advances_by_consumed_budget() {
    let mut rt = build(16, 7, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    for i in 1..=4 {
        let s = rt.step().unwrap();
        assert_eq!(s.time_after, GuestTicks::new(7 * i));
        assert_eq!(rt.time(), GuestTicks::new(7 * i));
    }
}

#[test]
fn round_robin_visits_units_in_id_order() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let ids: Vec<u64> = (0..6).map(|_| rt.step().unwrap().unit.raw()).collect();
    assert_eq!(ids, vec![0, 1, 2, 0, 1, 2]);
}

#[test]
fn finished_units_are_skipped() {
    let mut rt = build(16, 1, 100);
    // Unit 0 finishes after 2 steps, unit 1 finishes after 5.
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 2));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let mut visited = Vec::new();
    for _ in 0..7 {
        match rt.step() {
            Ok(s) => visited.push(s.unit.raw()),
            Err(StepError::NoRunnableUnit) => break,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    // Expected sequence: 0, 1, 0, 1, 1, 1, 1
    // (unit 0 takes 2 steps, then is finished; unit 1 takes 5)
    assert_eq!(visited, vec![0, 1, 0, 1, 1, 1, 1]);
    // After all units are finished, scheduler returns NoRunnableUnit.
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
}

#[test]
fn max_steps_cap_trips_deadlock_detector() {
    let mut rt = build(16, 1, 3);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 1000));
    assert!(rt.step().is_ok());
    assert!(rt.step().is_ok());
    assert!(rt.step().is_ok());
    assert_eq!(rt.step().unwrap_err(), StepError::MaxStepsExceeded);
    assert_eq!(rt.steps_taken(), 3);
}

#[test]
fn time_overflow_is_caught() {
    // Budget so large that two steps would push past u64::MAX.
    let mut rt = Runtime::new(GuestMemory::new(0), Budget::new(u64::MAX - 5), 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    // First step: time goes to u64::MAX - 5.
    let s = rt.step().unwrap();
    assert_eq!(s.time_after, GuestTicks::new(u64::MAX - 5));
    // Second step: would push past u64::MAX, caught.
    assert_eq!(rt.step().unwrap_err(), StepError::TimeOverflow);
}

#[test]
fn epoch_does_not_advance_in_this_slice() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    for _ in 0..3 {
        let s = rt.step().unwrap();
        assert_eq!(s.epoch_after, Epoch::ZERO);
    }
    assert_eq!(rt.epoch(), Epoch::ZERO);
}

#[test]
fn step_returns_emitted_effects_in_order() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let s = rt.step().unwrap();
    assert_eq!(s.result.emitted_effects.len(), 1);
    assert_eq!(
        s.result.emitted_effects[0],
        Effect::TraceMarker {
            marker: 1,
            source: UnitId::new(0),
        }
    );
}

/// A unit that emits one `SharedWriteIntent` per step against a
/// fixed range, then finishes after `max` steps.
struct WritingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl ExecutionUnit for WritingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        let bytes = vec![n as u8; 4];
        let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            emitted_effects: vec![Effect::SharedWriteIntent {
                range,
                bytes: WritePayload::new(bytes),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            }],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn step_emits_unit_scheduled_then_step_completed_in_order() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind, TracedYieldReason};
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.step().unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // UnitScheduled, StepCompleted, EffectEmitted (the trace marker
    // CountingUnit emits each step).
    assert_eq!(records.len(), 3);
    match records[2] {
        TraceRecord::EffectEmitted {
            unit,
            sequence,
            kind,
        } => {
            assert_eq!(unit, UnitId::new(0));
            assert_eq!(sequence, 0);
            assert_eq!(kind, TracedEffectKind::TraceMarker);
        }
        ref other => panic!("expected EffectEmitted, got {other:?}"),
    }
    match records[0] {
        TraceRecord::UnitScheduled {
            unit,
            granted_budget,
            time,
            epoch,
        } => {
            assert_eq!(unit, UnitId::new(0));
            assert_eq!(granted_budget, Budget::new(5));
            assert_eq!(time, GuestTicks::ZERO);
            assert_eq!(epoch, Epoch::ZERO);
        }
        ref other => panic!("expected UnitScheduled, got {other:?}"),
    }
    match records[1] {
        TraceRecord::StepCompleted {
            unit,
            yield_reason,
            consumed_budget,
            time_after,
        } => {
            assert_eq!(unit, UnitId::new(0));
            assert_eq!(yield_reason, TracedYieldReason::BudgetExhausted);
            assert_eq!(consumed_budget, Budget::new(5));
            assert_eq!(time_after, GuestTicks::new(5));
        }
        ref other => panic!("expected StepCompleted, got {other:?}"),
    }
}

#[test]
fn step_with_no_runnable_unit_emits_nothing() {
    let mut rt = build(16, 5, 100);
    assert!(rt.step().is_err());
    assert_eq!(rt.trace().record_count(), 0);
    assert_eq!(rt.trace().byte_len(), 0);
}

#[test]
fn deadlock_trip_emits_nothing_for_the_failed_step() {
    let mut rt = build(16, 1, 1);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.step().unwrap(); // UnitScheduled, StepCompleted, EffectEmitted
    let count_before = rt.trace().record_count();
    assert_eq!(count_before, 3);
    // Second step trips the cap before the schedule decision; no
    // new records.
    assert_eq!(rt.step().unwrap_err(), StepError::MaxStepsExceeded);
    assert_eq!(rt.trace().record_count(), count_before);
}

#[test]
fn finished_yield_reason_is_traced_as_finished() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedYieldReason};
    let mut rt = build(16, 1, 100);
    // CountingUnit::new(_, 1) finishes on its first step.
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.step().unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let step_record = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .find(|r| matches!(r, TraceRecord::StepCompleted { .. }))
        .expect("StepCompleted present");
    match step_record {
        TraceRecord::StepCompleted { yield_reason, .. } => {
            assert_eq!(yield_reason, TracedYieldReason::Finished);
        }
        other => panic!("expected StepCompleted, got {other:?}"),
    }
}

#[test]
fn level_filter_drops_scheduling_records() {
    use cellgov_trace::{TraceLevel, TraceReader, TraceRecord, TraceWriter};
    // Writer that only records commits -- scheduling records drop.
    let writer = TraceWriter::with_levels(&[TraceLevel::Commits]);
    let mut rt = Runtime::with_trace_writer(GuestMemory::new(16), Budget::new(1), 100, writer);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 2,
    });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // Only the CommitApplied record survived the filter.
    assert_eq!(records.len(), 1);
    assert!(matches!(records[0], TraceRecord::CommitApplied { .. }));
}

#[test]
fn step_then_commit_emits_commit_applied_with_post_epoch() {
    use cellgov_trace::{HashCheckpointKind, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 3,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // UnitScheduled, StepCompleted, EffectEmitted (one
    // SharedWriteIntent), CommitApplied, four StateHashCheckpoints
    // (CommittedMemory, RunnableQueue, UnitStatus, SyncState).
    assert_eq!(records.len(), 8);
    match records[4] {
        TraceRecord::StateHashCheckpoint { kind, .. } => {
            assert_eq!(kind, HashCheckpointKind::CommittedMemory);
        }
        ref other => panic!("expected CommittedMemory checkpoint, got {other:?}"),
    }
    match records[5] {
        TraceRecord::StateHashCheckpoint { kind, .. } => {
            assert_eq!(kind, HashCheckpointKind::RunnableQueue);
        }
        ref other => panic!("expected RunnableQueue checkpoint, got {other:?}"),
    }
    match records[6] {
        TraceRecord::StateHashCheckpoint { kind, .. } => {
            assert_eq!(kind, HashCheckpointKind::UnitStatus);
        }
        ref other => panic!("expected UnitStatus checkpoint, got {other:?}"),
    }
    match records[7] {
        TraceRecord::StateHashCheckpoint { kind, .. } => {
            assert_eq!(kind, HashCheckpointKind::SyncState);
        }
        ref other => panic!("expected SyncState checkpoint, got {other:?}"),
    }
    match records[3] {
        TraceRecord::CommitApplied {
            unit,
            writes_committed,
            effects_deferred,
            fault_discarded,
            epoch_after,
        } => {
            assert_eq!(unit, UnitId::new(0));
            assert_eq!(writes_committed, 1);
            assert_eq!(effects_deferred, 0);
            assert!(!fault_discarded);
            assert_eq!(epoch_after, Epoch::new(1));
        }
        ref other => panic!("expected CommitApplied, got {other:?}"),
    }
}

#[test]
fn step_emits_one_effect_record_per_effect_in_emission_order() {
    use cellgov_effects::{Effect, WritePayload};
    use cellgov_event::PriorityClass;
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};

    // A unit that emits three effects of different kinds in a
    // fixed order. The trace must contain three EffectEmitted
    // records with sequence 0..2 in the same order, sandwiched
    // between StepCompleted and CommitApplied.
    struct MultiEffectUnit {
        id: UnitId,
        done: Cell<bool>,
    }
    impl ExecutionUnit for MultiEffectUnit {
        type Snapshot = ();
        fn unit_id(&self) -> UnitId {
            self.id
        }
        fn status(&self) -> UnitStatus {
            if self.done.get() {
                UnitStatus::Finished
            } else {
                UnitStatus::Runnable
            }
        }
        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
        ) -> ExecutionStepResult {
            self.done.set(true);
            let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                emitted_effects: vec![
                    Effect::TraceMarker {
                        marker: 1,
                        source: self.id,
                    },
                    Effect::SharedWriteIntent {
                        range,
                        bytes: WritePayload::new(vec![1, 2, 3, 4]),
                        ordering: PriorityClass::Normal,
                        source: self.id,
                        source_time: GuestTicks::ZERO,
                    },
                    Effect::TraceMarker {
                        marker: 2,
                        source: self.id,
                    },
                ],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            }
        }
        fn snapshot(&self) {}
    }

    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| MultiEffectUnit {
        id,
        done: Cell::new(false),
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let effects: Vec<(u32, TracedEffectKind)> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::EffectEmitted { sequence, kind, .. } => Some((sequence, kind)),
            _ => None,
        })
        .collect();
    assert_eq!(
        effects,
        vec![
            (0, TracedEffectKind::TraceMarker),
            (1, TracedEffectKind::SharedWriteIntent),
            (2, TracedEffectKind::TraceMarker),
        ]
    );
}

#[test]
fn effect_records_are_filtered_by_level() {
    use cellgov_trace::{TraceLevel, TraceReader, TraceRecord};
    // Writer that records only Scheduling -- effect records drop.
    let writer = TraceWriter::with_levels(&[TraceLevel::Scheduling]);
    let mut rt = Runtime::with_trace_writer(GuestMemory::new(16), Budget::new(1), 100, writer);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 3));
    rt.step().unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let any_effect_record = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .any(|r| matches!(r, TraceRecord::EffectEmitted { .. }));
    assert!(!any_effect_record);
}

#[test]
fn commit_emits_state_hash_checkpoint_after_commit_applied() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 1,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // The hash checkpoint must come immediately after CommitApplied
    // so replay tooling sees a strict commit -> hash sequence.
    let commit_idx = records
        .iter()
        .position(|r| matches!(r, TraceRecord::CommitApplied { .. }))
        .expect("CommitApplied present");
    match records.get(commit_idx + 1) {
        Some(TraceRecord::StateHashCheckpoint { kind, hash }) => {
            assert_eq!(*kind, HashCheckpointKind::CommittedMemory);
            // Hash matches a freshly computed hash of memory.
            assert_eq!(*hash, StateHash::new(rt.memory().content_hash()));
        }
        other => panic!("expected StateHashCheckpoint after CommitApplied, got {other:?}"),
    }
}

#[test]
fn committed_memory_state_hash_changes_after_write() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 3,
    });
    // Two successful commits: WritingUnit writes [1,1,1,1] then
    // [2,2,2,2] to addr 0, so the two CommittedMemory checkpoints
    // must differ.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result).unwrap();
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
}

#[test]
fn sync_state_checkpoint_changes_when_a_mailbox_is_registered() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    // Two runtimes that differ only in whether a mailbox is
    // registered. CountingUnit doesn't touch mailboxes, so the
    // committed memory and unit status hashes are identical
    // across the two runs; only the SyncState checkpoint can
    // differ.
    fn run(register_mailbox: bool) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        if register_mailbox {
            let _ = rt.mailbox_registry_mut().register();
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    let no_mb = run(false);
    let one_mb = run(true);
    assert_ne!(no_mb, one_mb);
}

#[test]
fn dma_completion_fires_and_applies_transfer() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    // Seed committed memory with source data, enqueue a DMA
    // completion scheduled for time 5, then step+commit past it.
    let mut rt = build(256, 5, 100);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0xaa, 0xbb, 0xcc, 0xdd],
        )
        .unwrap();
    // Enqueue a completion directly (bypasses the Effect path;
    // tests the firing logic in isolation).
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    // Step consumes budget=5, time goes to 5. Commit fires the
    // completion (scheduled at 3, now=5 >= 3).
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result).unwrap();
    assert_eq!(outcome.dma_completions_fired, 1);
    // Destination now has the source bytes.
    assert_eq!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
            .unwrap(),
        &[0xaa, 0xbb, 0xcc, 0xdd]
    );
}

#[test]
fn dma_completion_wakes_issuer() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 5, 100);
    // Register two units. Block unit 1 (the DMA issuer) manually.
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Blocked);
    // Enqueue a completion from unit 1.
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(1),
    )
    .unwrap();
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    // Step runs unit 0 (unit 1 is blocked). Time -> 5.
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(0));
    let outcome = rt.commit_step(&s.result).unwrap();
    assert_eq!(outcome.dma_completions_fired, 1);
    // Unit 1 is now woken (override set to Runnable).
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(cellgov_exec::UnitStatus::Runnable)
    );
}

#[test]
fn dma_completion_does_not_fire_before_its_time() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 2, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    // Scheduled at time 100, budget=2 so first step reaches time=2.
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(100)), None);
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result).unwrap();
    // Not yet due.
    assert_eq!(outcome.dma_completions_fired, 0);
    assert_eq!(rt.dma_queue().len(), 1);
}

#[test]
fn sync_state_checkpoint_changes_when_a_signal_register_value_changes() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    // Two runtimes, identical except one has a signal register
    // with a non-zero value. The mailbox registry, committed
    // memory, and unit status hashes are all identical -- the
    // SyncState checkpoint must still differ because it folds in
    // signal state.
    fn run(or_in_value: u32) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        let sig = rt.signal_registry_mut().register();
        if or_in_value != 0 {
            rt.signal_registry_mut()
                .get_mut(sig)
                .unwrap()
                .or_in(or_in_value);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    assert_ne!(run(0), run(0xa5));
}

#[test]
fn sync_state_checkpoint_changes_when_a_message_lands_in_a_mailbox() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    // Same fixture run twice, but the second run pre-seeds the
    // mailbox with a message. SyncState hash must differ.
    fn run(seed_message: Option<u32>) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        let mb_id = rt.mailbox_registry_mut().register();
        if let Some(word) = seed_message {
            rt.mailbox_registry_mut().get_mut(mb_id).unwrap().send(word);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result).unwrap();
        let bytes = rt.trace().bytes().to_vec();
        TraceReader::new(&bytes)
            .map(|r| r.expect("decode"))
            .find_map(|r| match r {
                TraceRecord::StateHashCheckpoint {
                    kind: HashCheckpointKind::SyncState,
                    hash,
                } => Some(hash),
                _ => None,
            })
            .expect("SyncState checkpoint present")
    }
    assert_ne!(run(None), run(Some(0xdead_beef)));
}

#[test]
fn unit_status_state_hash_changes_when_unit_finishes() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
    // CountingUnit::new(_, 2) starts Runnable, finishes after step 2.
    // The first commit's UnitStatus checkpoint sees Runnable; the
    // second sees Finished. The hashes must differ. CountingUnit
    // doesn't emit SharedWriteIntents so the CommittedMemory hash
    // stays constant -- this test isolates the UnitStatus signal.
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 2));
    for _ in 0..2 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result).unwrap();
    }
    let bytes = rt.trace().bytes().to_vec();
    let status_hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    let mem_hashes: Vec<StateHash> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash,
            } => Some(hash),
            _ => None,
        })
        .collect();
    assert_eq!(status_hashes.len(), 2);
    assert_ne!(status_hashes[0], status_hashes[1]);
    // Memory unchanged across both commits.
    assert_eq!(mem_hashes.len(), 2);
    assert_eq!(mem_hashes[0], mem_hashes[1]);
}

#[test]
fn commit_validation_failure_traces_as_fault_discarded() {
    use cellgov_effects::WritePayload;
    use cellgov_event::PriorityClass;
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_trace::{TraceReader, TraceRecord};

    // A unit that emits one out-of-range write -- the commit
    // pipeline rejects the batch and the runtime traces it as
    // fault_discarded with zero counts.
    struct OobUnit {
        id: UnitId,
        done: Cell<bool>,
    }
    impl ExecutionUnit for OobUnit {
        type Snapshot = ();
        fn unit_id(&self) -> UnitId {
            self.id
        }
        fn status(&self) -> UnitStatus {
            if self.done.get() {
                UnitStatus::Finished
            } else {
                UnitStatus::Runnable
            }
        }
        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
        ) -> ExecutionStepResult {
            self.done.set(true);
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                emitted_effects: vec![Effect::SharedWriteIntent {
                    // Range starts past end of memory -- definitely OOB.
                    range: ByteRange::new(GuestAddr::new(1024), 4).unwrap(),
                    bytes: WritePayload::new(vec![0; 4]),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                }],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            }
        }
        fn snapshot(&self) {}
    }

    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| OobUnit {
        id,
        done: Cell::new(false),
    });
    let s = rt.step().unwrap();
    let err = rt.commit_step(&s.result).unwrap_err();
    // We don't assert on the specific CommitError variant -- the
    // commit module already does that. Just verify it's an Err.
    let _ = err;
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    let commit_record = records
        .iter()
        .find(|r| matches!(r, TraceRecord::CommitApplied { .. }))
        .expect("CommitApplied present");
    match commit_record {
        TraceRecord::CommitApplied {
            unit,
            writes_committed,
            effects_deferred,
            fault_discarded,
            epoch_after,
        } => {
            assert_eq!(*unit, UnitId::new(0));
            assert_eq!(*writes_committed, 0);
            assert_eq!(*effects_deferred, 0);
            assert!(*fault_discarded);
            // Epoch advances on every commit boundary, including
            // failed ones.
            assert_eq!(*epoch_after, Epoch::new(1));
        }
        _ => unreachable!(),
    }
}

#[test]
fn trace_is_deterministic_across_two_identical_runs() {
    // Two runtimes built identically must produce byte-identical
    // traces. This validates the core invariant: replay determinism.
    fn run() -> Vec<u8> {
        let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
        rt.registry_mut().register_with(|id| WritingUnit {
            id,
            steps: Cell::new(0),
            max: 4,
        });
        for _ in 0..4 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result).unwrap();
        }
        rt.trace().bytes().to_vec()
    }
    let a = run();
    let b = run();
    assert_eq!(a, b);
}

#[test]
fn step_then_commit_writes_become_visible() {
    let mut rt = build(16, 1, 100);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 5,
    });
    // First step: writes [1,1,1,1] to addr 0.
    let s1 = rt.step().unwrap();
    let outcome1 = rt.commit_step(&s1.result).unwrap();
    assert_eq!(outcome1.writes_committed, 1);
    assert!(!outcome1.fault_discarded);
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[1, 1, 1, 1]
    );
    // Epoch advanced by exactly one.
    assert_eq!(rt.epoch(), Epoch::new(1));

    // Second step: overwrites with [2,2,2,2].
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result).unwrap();
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[2, 2, 2, 2]
    );
    assert_eq!(rt.epoch(), Epoch::new(2));
}
