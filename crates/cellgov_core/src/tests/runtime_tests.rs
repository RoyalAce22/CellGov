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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.max {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::TraceMarker {
            marker: n as u32,
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
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
fn step_with_all_units_blocked_returns_all_blocked() {
    // Registry is non-empty but every unit has a Blocked
    // status override. The runtime must distinguish this from
    // the empty-registry case so callers can tell "nothing will
    // wake" from "everyone parked on some external signal."
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.registry_mut()
        .set_status_override(UnitId::new(0), cellgov_exec::UnitStatus::Blocked);
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Blocked);
    assert_eq!(rt.step().unwrap_err(), StepError::AllBlocked);
    assert_eq!(rt.steps_taken(), 0);
}

#[test]
fn step_with_all_finished_returns_no_runnable_not_all_blocked() {
    // Finished units are terminal -- they will never wake. That
    // must read as NoRunnableUnit (terminal stall), not
    // AllBlocked (soft stall).
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    // Exhaust the unit.
    assert!(rt.step().is_ok());
    // Now unit 0 is Finished. Next step should report
    // NoRunnableUnit, not AllBlocked.
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
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
    let s = rt.step().unwrap();
    assert_eq!(s.time_after, GuestTicks::new(u64::MAX - 5));
    assert_eq!(rt.step().unwrap_err(), StepError::TimeOverflow);
}

#[test]
fn epoch_does_not_advance_within_a_single_step() {
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
    assert_eq!(s.effects.len(), 1);
    assert_eq!(
        s.effects[0],
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
        effects: &mut Vec<Effect>,
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
        effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(bytes),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
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
    // UnitScheduled, StepCompleted, EffectEmitted.
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
    rt.step().unwrap();
    let count_before = rt.trace().record_count();
    assert_eq!(count_before, 3);
    // Second step trips the cap before the schedule decision.
    assert_eq!(rt.step().unwrap_err(), StepError::MaxStepsExceeded);
    assert_eq!(rt.trace().record_count(), count_before);
}

#[test]
fn finished_yield_reason_is_traced_as_finished() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedYieldReason};
    let mut rt = build(16, 1, 100);
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
    let writer = TraceWriter::with_levels(&[TraceLevel::Commits]);
    let mut rt = Runtime::with_trace_writer(GuestMemory::new(16), Budget::new(1), 100, writer);
    rt.registry_mut().register_with(|id| WritingUnit {
        id,
        steps: Cell::new(0),
        max: 2,
    });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
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
    rt.commit_step(&s.result, &s.effects).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // UnitScheduled, StepCompleted, EffectEmitted, CommitApplied,
    // then CommittedMemory / RunnableQueue / UnitStatus / SyncState.
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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.done.set(true);
            let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
            effects.push(Effect::TraceMarker {
                marker: 1,
                source: self.id,
            });
            effects.push(Effect::SharedWriteIntent {
                range,
                bytes: WritePayload::new(vec![1, 2, 3, 4]),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            });
            effects.push(Effect::TraceMarker {
                marker: 2,
                source: self.id,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
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
    rt.commit_step(&s.result, &s.effects).unwrap();
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
    rt.commit_step(&s.result, &s.effects).unwrap();
    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    // Checkpoint must come immediately after CommitApplied so replay
    // tooling sees a strict commit -> hash sequence.
    let commit_idx = records
        .iter()
        .position(|r| matches!(r, TraceRecord::CommitApplied { .. }))
        .expect("CommitApplied present");
    match records.get(commit_idx + 1) {
        Some(TraceRecord::StateHashCheckpoint { kind, hash }) => {
            assert_eq!(*kind, HashCheckpointKind::CommittedMemory);
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
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
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
    fn run(register_mailbox: bool) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        if register_mailbox {
            let _ = rt.mailbox_registry_mut().register();
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
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
    let mut rt = build(256, 5, 100);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0xaa, 0xbb, 0xcc, 0xdd],
        )
        .unwrap();
    // Enqueue directly (bypasses the Effect path) to exercise firing in isolation.
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
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.dma_completions_fired, 1);
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
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Blocked);
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(1),
    )
    .unwrap();
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(0));
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.dma_completions_fired, 1);
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
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.dma_completions_fired, 0);
    assert_eq!(rt.dma_queue().len(), 1);
}

#[test]
fn sync_state_checkpoint_changes_when_a_signal_register_value_changes() {
    use cellgov_trace::{HashCheckpointKind, StateHash, TraceReader, TraceRecord};
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
        rt.commit_step(&s.result, &s.effects).unwrap();
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
    fn run(seed_message: Option<u32>) -> StateHash {
        let mut rt = build(16, 1, 100);
        rt.registry_mut()
            .register_with(|id| CountingUnit::new(id, 1));
        let mb_id = rt.mailbox_registry_mut().register();
        if let Some(word) = seed_message {
            rt.mailbox_registry_mut().get_mut(mb_id).unwrap().send(word);
        }
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
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
    // CountingUnit emits no SharedWriteIntents so CommittedMemory
    // hash stays constant; this isolates the UnitStatus signal.
    let mut rt = build(16, 1, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 2));
    for _ in 0..2 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
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
    assert_eq!(mem_hashes.len(), 2);
    assert_eq!(mem_hashes[0], mem_hashes[1]);
}

#[test]
fn commit_validation_failure_traces_as_fault_discarded() {
    use cellgov_effects::WritePayload;
    use cellgov_event::PriorityClass;
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_trace::{TraceReader, TraceRecord};

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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.done.set(true);
            effects.push(Effect::SharedWriteIntent {
                // Range starts past end of 16-byte memory.
                range: ByteRange::new(GuestAddr::new(1024), 4).unwrap(),
                bytes: WritePayload::new(vec![0; 4]),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
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
    let err = rt.commit_step(&s.result, &s.effects).unwrap_err();
    // CommitError variants are exercised in commit_tests; here we
    // just need an Err of any kind to drive the trace path.
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
            // Epoch advances on every commit boundary, including failures.
            assert_eq!(*epoch_after, Epoch::new(1));
        }
        _ => unreachable!(),
    }
}

#[test]
fn trace_is_deterministic_across_two_identical_runs() {
    fn run() -> Vec<u8> {
        let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
        rt.registry_mut().register_with(|id| WritingUnit {
            id,
            steps: Cell::new(0),
            max: 4,
        });
        for _ in 0..4 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
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
    let s1 = rt.step().unwrap();
    let outcome1 = rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(outcome1.writes_committed, 1);
    assert!(!outcome1.fault_discarded);
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[1, 1, 1, 1]
    );
    assert_eq!(rt.epoch(), Epoch::new(1));

    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.memory()
            .read(cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap())
            .unwrap(),
        &[2, 2, 2, 2]
    );
    assert_eq!(rt.epoch(), Epoch::new(2));
}

#[test]
fn fault_driven_mode_skips_trace_records() {
    use cellgov_trace::TraceReader;

    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(10), 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    let reader = TraceReader::new(rt.trace().bytes());
    let records: Vec<_> = reader.collect();
    assert!(
        records.is_empty(),
        "FaultDriven mode should emit no trace records, got {}",
        records.len()
    );
}

#[test]
fn full_trace_mode_emits_trace_records() {
    use cellgov_trace::TraceReader;

    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(10), 100);
    assert_eq!(rt.mode(), RuntimeMode::FullTrace);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    let reader = TraceReader::new(rt.trace().bytes());
    let records: Vec<_> = reader.collect();
    // UnitScheduled + StepCompleted + CommitApplied + 4 hash checkpoints = 7 min.
    assert!(
        records.len() >= 7,
        "FullTrace mode should emit >= 7 trace records, got {}",
        records.len()
    );
}

#[test]
fn max_steps_zero_rejects_first_step() {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(10), 0);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    assert_eq!(rt.step(), Err(StepError::MaxStepsExceeded));
}

/// Simulates per-step state-hash production; runtime drains the
/// configured pairs after run_until_yield and emits one
/// TraceRecord::PpuStateHash per pair with monotonically
/// incrementing step indices across calls.
type FullStateTuple = (u64, [u64; 32], u64, u64, u64, u32);

struct StateHashEmittingUnit {
    id: UnitId,
    pairs_per_step: Vec<Vec<(u64, u64)>>,
    /// Zoom-in snapshots paired with `pairs_per_step`; empty inner
    /// Vec means no full-state records that step.
    full_per_step: Vec<Vec<FullStateTuple>>,
    step_idx: Cell<usize>,
}

impl ExecutionUnit for StateHashEmittingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.step_idx.get() >= self.pairs_per_step.len() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.step_idx.set(self.step_idx.get() + 1);
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) -> Self::Snapshot {}
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        let i = self.step_idx.get();
        if i == 0 || i > self.pairs_per_step.len() {
            return vec![];
        }
        self.pairs_per_step[i - 1].clone()
    }
    fn drain_retired_state_full(&mut self) -> Vec<FullStateTuple> {
        let i = self.step_idx.get();
        if i == 0 || i > self.full_per_step.len() {
            return vec![];
        }
        self.full_per_step[i - 1].clone()
    }
}

#[test]
fn runtime_emits_ppu_state_hash_records_with_monotonic_step_index() {
    use cellgov_trace::{StateHash, TraceReader, TraceRecord};
    let mut rt = build(16, 5, 100);
    rt.registry_mut().register_with(|id| StateHashEmittingUnit {
        id,
        pairs_per_step: vec![
            vec![(0x100, 0xaaa), (0x104, 0xbbb)],
            vec![(0x200, 0xccc)],
            vec![],
        ],
        full_per_step: vec![vec![], vec![], vec![]],
        step_idx: Cell::new(0),
    });
    rt.step().unwrap();
    rt.step().unwrap();
    rt.step().unwrap();

    let bytes = rt.trace().bytes().to_vec();
    let hashes: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::PpuStateHash { .. }))
        .collect();

    assert_eq!(
        hashes.len(),
        3,
        "3 retired-instruction fingerprints in total"
    );

    let extract = |r: &TraceRecord| match r {
        TraceRecord::PpuStateHash { step, pc, hash } => (*step, *pc, hash.raw()),
        _ => panic!("expected PpuStateHash"),
    };
    assert_eq!(extract(&hashes[0]), (0, 0x100, 0xaaa));
    assert_eq!(extract(&hashes[1]), (1, 0x104, 0xbbb));
    assert_eq!(extract(&hashes[2]), (2, 0x200, 0xccc));
    let _ = StateHash::new(0); // keep the import live even when filtered out
}

#[test]
fn runtime_emits_no_ppu_state_hash_when_unit_drains_empty() {
    use cellgov_trace::{TraceReader, TraceRecord};
    // CountingUnit uses the trait default (empty drain).
    let mut rt = build(16, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    for _ in 0..3 {
        rt.step().unwrap();
    }
    let bytes = rt.trace().bytes().to_vec();
    let count = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::PpuStateHash { .. }))
        .count();
    assert_eq!(count, 0);
}

#[test]
fn runtime_routes_full_states_to_zoom_trace_not_main_trace() {
    use cellgov_trace::{TraceReader, TraceRecord};
    let mut rt = build(16, 5, 100);
    rt.registry_mut().register_with(|id| StateHashEmittingUnit {
        id,
        // Three retired instructions; only the middle one is inside the zoom window.
        pairs_per_step: vec![vec![(0x100, 0xaa), (0x104, 0xbb), (0x108, 0xcc)]],
        full_per_step: vec![vec![(0x104, [0u64; 32], 0, 0, 0, 0)]],
        step_idx: Cell::new(0),
    });
    rt.step().unwrap();

    let main_bytes = rt.trace().bytes().to_vec();
    let main_records: Vec<_> = TraceReader::new(&main_bytes)
        .map(|r| r.expect("decode"))
        .collect();
    let main_hashes = main_records
        .iter()
        .filter(|r| matches!(r, TraceRecord::PpuStateHash { .. }))
        .count();
    let main_fulls = main_records
        .iter()
        .filter(|r| matches!(r, TraceRecord::PpuStateFull { .. }))
        .count();
    assert_eq!(main_hashes, 3, "all hashes go to main stream");
    assert_eq!(main_fulls, 0, "full states never appear in main stream");

    let zoom_bytes = rt.zoom_trace().bytes().to_vec();
    let zoom_records: Vec<_> = TraceReader::new(&zoom_bytes)
        .map(|r| r.expect("decode"))
        .collect();
    assert_eq!(zoom_records.len(), 1);
    match &zoom_records[0] {
        TraceRecord::PpuStateFull { pc, .. } => assert_eq!(*pc, 0x104),
        other => panic!("expected PpuStateFull, got {other:?}"),
    }
}

#[test]
fn into_memory_returns_committed_state() {
    let mut mem = GuestMemory::new(64);
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();

    let rt = Runtime::new(mem, Budget::new(1), 100);
    let recovered = rt.into_memory();

    assert_eq!(&recovered.as_bytes()[0..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(recovered.size(), 64);
}

// Trivial-commit fast path -- observable-contract preservation.

/// Unit that emits zero effects every step and finishes after
/// `max` steps; exactly the shape the trivial-commit fast path
/// short-circuits.
struct SilentUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
}

impl ExecutionUnit for SilentUnit {
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
        _effects: &mut Vec<Effect>,
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
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn commit_fast_path_empty_loop_advances_epoch_monotonically() {
    let mut rt = build(64, 1, 20_000);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Finished yields take the slow path; BudgetExhausted does not.
    // Give the unit enough runway so all 10K steps are BudgetExhausted.
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100_000,
    });

    let start_epoch = rt.epoch();
    for _ in 0..10_000 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    assert_eq!(
        rt.epoch().raw(),
        start_epoch.raw() + 10_000,
        "epoch must advance exactly once per commit, even on the fast path"
    );
    assert!(
        rt.trace().bytes().is_empty(),
        "FaultDriven + empty-effect steps must produce no trace records"
    );
}

#[test]
fn commit_fast_path_defers_to_slow_path_when_dma_pending() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};

    let mut rt = build(256, 1, 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0x11, 0x22, 0x33, 0x44],
        )
        .unwrap();
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    // Scheduled at tick 3; budget=1 so step 3 first reaches accumulated time=3.
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100,
    });

    let s = rt.step().unwrap();
    let o1 = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o1.dma_completions_fired, 0);
    let s = rt.step().unwrap();
    let o2 = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o2.dma_completions_fired, 0);
    let s = rt.step().unwrap();
    let o3 = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        o3.dma_completions_fired, 1,
        "DMA must fire at its scheduled tick despite silent steps"
    );
    assert_eq!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
            .unwrap(),
        &[0x11, 0x22, 0x33, 0x44]
    );
    // Queue empty again: fast path re-engages; epoch still advances.
    let epoch_before = rt.epoch();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.epoch().raw(), epoch_before.raw() + 1);
}

/// Wake visibility is carried by `status_overrides`, which the
/// fast path does not touch, so a DMA-completion wake on one
/// unit stays observable through another unit's silent steps.
#[test]
fn commit_fast_path_preserves_wake_visibility_through_silent_steps() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};

    let mut rt = build(256, 1, 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Unit 0 is the DMA-issuing waiter; starts Blocked.
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100,
    });
    rt.registry_mut()
        .set_status_override(UnitId::new(0), UnitStatus::Blocked);
    // Unit 1 drives the clock with silent steps.
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100,
    });
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(2)), None);

    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(1));
    let o = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o.dma_completions_fired, 0);
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Blocked)
    );
    // Step 2: unit 1 runs again, DMA fires, unit 0 wakes.
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(1));
    let o = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o.dma_completions_fired, 1);
    let wake_epoch = rt.epoch();
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Runnable),
        "DMA completion must wake the issuer"
    );
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        rt.epoch().raw(),
        wake_epoch.raw() + 1,
        "epoch must advance once per commit, fast or slow"
    );
}

/// Emits `ReservationAcquire` then `SharedWriteIntent` to the
/// same line across two steps; drives sync_state_hash folding
/// through a scripted acquire / release sequence.
struct ReservationDriverUnit {
    id: UnitId,
    steps: Cell<u64>,
    line_addr: u64,
}

impl ExecutionUnit for ReservationDriverUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        match n {
            1 => {
                effects.push(Effect::ReservationAcquire {
                    line_addr: self.line_addr,
                    source: self.id,
                });
            }
            2 => {
                let range = ByteRange::new(GuestAddr::new(self.line_addr), 4).unwrap();
                effects.push(Effect::SharedWriteIntent {
                    range,
                    bytes: WritePayload::new(vec![0xAA; 4]),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
            }
            _ => {}
        }
        let yield_reason = if n >= 2 {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Writes FIFO bytes encoding `GCM_FLIP_COMMAND` with the given
/// buffer index and advances put via the RSX control-register
/// mirror; finishes after one step.
struct RsxFlipCommandEmitterUnit {
    id: UnitId,
    steps: Cell<u64>,
    fifo_base: u32,
    buffer_index: u32,
}

impl ExecutionUnit for RsxFlipCommandEmitterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use crate::rsx::method::{GCM_FLIP_COMMAND, NV_COUNT_SHIFT};
        use crate::rsx::RSX_CONTROL_PUT_ADDR;
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        self.steps.set(1);
        // FIFO: GCM_FLIP_COMMAND header (count=1) + arg.
        let header: u32 = (1u32 << NV_COUNT_SHIFT) | (GCM_FLIP_COMMAND as u32);
        let mut fifo_bytes: Vec<u8> = Vec::with_capacity(8);
        fifo_bytes.extend_from_slice(&header.to_le_bytes());
        fifo_bytes.extend_from_slice(&self.buffer_index.to_le_bytes());
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(self.fifo_base as u64), 8).unwrap(),
            bytes: WritePayload::new(fifo_bytes),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(RSX_CONTROL_PUT_ADDR as u64), 4).unwrap(),
            bytes: WritePayload::new((self.fifo_base + 8).to_be_bytes().to_vec()),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn rsx_flip_waiting_observable_between_two_commits_then_done() {
    // Three-batch state machine: batch 1 queues RsxFlipRequest
    // (flip stays DONE). Batch 2 applies it (WAITING + pending).
    // Batch 3 observes pending_at_entry=true and fires DONE.
    use crate::rsx::flip::{
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
    };
    const FIFO_BASE: u32 = 0x200;
    let mut rt = build_with_rsx_and_label_region(0x4000);
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut()
        .register_with(|id| RsxFlipCommandEmitterUnit {
            id,
            steps: Cell::new(0),
            fifo_base: FIFO_BASE,
            buffer_index: 2,
        });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 1 end: effect queued, not yet applied; flip still DONE"
    );
    assert!(!rt.rsx_flip().pending());
    // Primary unit is Finished; need a live unit so rt.step() does not hit NoRunnableUnit.
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
        "batch 2 end: RsxFlipRequest applied; WAITING observable"
    );
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 2);
    let s3 = rt.step().unwrap();
    rt.commit_step(&s3.result, &s3.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 3 end: transition fired"
    );
    assert!(!rt.rsx_flip().pending());
}

/// Emits a single `RsxFlipRequest` effect on its first step.
/// Skips the FIFO drain (which adds a one-batch delay) to exercise
/// the commit pipeline's RsxFlipRequest application path directly.
struct RsxFlipRequestEmitterUnit {
    id: UnitId,
    steps: Cell<u64>,
    buffer_index: u8,
}

impl ExecutionUnit for RsxFlipRequestEmitterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps.set(1);
        effects.push(Effect::RsxFlipRequest {
            buffer_index: self.buffer_index,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn rsx_flip_request_applied_same_batch_does_not_immediately_transition() {
    // Pins the one-batch-delay contract: a RsxFlipRequest applied
    // in this commit does not fire DONE in the same batch because
    // pending_at_entry was false.
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_WAITING;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 1,
        });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
        "WAITING observable; DONE transition does NOT fire same-batch"
    );
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);
}

#[test]
fn rsx_flip_transitions_to_done_on_next_commit_boundary() {
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 2,
        });
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert!(rt.rsx_flip().pending(), "batch 1: WAITING + pending");
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 2: DONE transition fired"
    );
    assert!(!rt.rsx_flip().pending());
}

#[test]
fn rsx_flip_second_request_while_pending_resolves_one_transition() {
    // A second RsxFlipRequest arriving while pending updates
    // buffer_index but does NOT add a second transition: exactly
    // one WAITING -> DONE fires for the whole sequence.
    use crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
    let mut rt = build(4096, 1, 100);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 1,
        });
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 5,
        });
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);
    // Second request applies (buffer=5), then DONE transition
    // fires because pending_at_entry was true.
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "exactly one WAITING -> DONE transition for the request sequence"
    );
    assert!(!rt.rsx_flip().pending());
    assert_eq!(
        rt.rsx_flip().buffer_index(),
        5,
        "second request's buffer_index remains recorded"
    );
}

/// Writes a u32 to a configurable RSX control-register slot via a
/// single SharedWriteIntent on its first step. The slot address
/// and value are constructor params so tests can target put / get
/// / reference individually.
struct RsxControlWriterUnit {
    id: UnitId,
    steps: Cell<u64>,
    slot_addr: u64,
    value: u32,
}

impl ExecutionUnit for RsxControlWriterUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 1 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        self.steps.set(1);
        let range = ByteRange::new(GuestAddr::new(self.slot_addr), 4).unwrap();
        effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(self.value.to_be_bytes().to_vec()),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build_with_rsx_writable() -> Runtime {
    use cellgov_mem::{GuestMemory, PageSize, Region};
    let regions = vec![
        Region::new(0, 0x1000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("regions non-overlapping");
    Runtime::new(mem, Budget::new(1), 100)
}

#[test]
fn rsx_mirror_writes_disabled_by_default() {
    let rt = build(4096, 1, 100);
    assert!(!rt.rsx_mirror_writes_enabled());
}

#[test]
fn rsx_mirror_writes_off_leaves_cursor_unchanged() {
    use crate::rsx::RSX_CONTROL_PUT_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_PUT_ADDR as u64,
        value: 0x1234,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0, "mirror off; cursor untouched");
}

#[test]
fn rsx_mirror_writes_on_routes_put_to_cursor() {
    use crate::rsx::RSX_CONTROL_PUT_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_PUT_ADDR as u64,
        value: 0x0000_1000,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0x0000_1000);
    // Memory holds the big-endian value so a guest read-back sees its own store.
    use cellgov_mem::{ByteRange, GuestAddr};
    let mem_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(RSX_CONTROL_PUT_ADDR as u64), 4).unwrap())
        .unwrap();
    assert_eq!(mem_bytes, &0x0000_1000u32.to_be_bytes());
}

#[test]
fn rsx_mirror_writes_on_routes_get_to_cursor() {
    use crate::rsx::RSX_CONTROL_GET_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_GET_ADDR as u64,
        value: 0x0000_2000,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().get(), 0x0000_2000);
}

#[test]
fn rsx_mirror_writes_on_routes_reference_to_cursor() {
    use crate::rsx::RSX_CONTROL_REF_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_REF_ADDR as u64,
        value: 0xCAFE_BABE,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().current_reference(), 0xCAFE_BABE);
}

/// Drives an RSX label-write round trip: step 1 writes an
/// OFFSET + RELEASE method pair and advances put; step 2 drains
/// the pending RsxLabelWrite into memory at `label_base + offset`.
struct RsxOffsetReleaseDriverUnit {
    id: UnitId,
    steps: Cell<u64>,
    fifo_base: u32,
    put_target: u32,
    sem_offset: u32,
    release_value: u32,
}

impl ExecutionUnit for RsxOffsetReleaseDriverUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use crate::rsx::method::{
            NV406E_SEMAPHORE_OFFSET, NV406E_SEMAPHORE_RELEASE, NV_COUNT_SHIFT,
        };
        use crate::rsx::RSX_CONTROL_PUT_ADDR;
        use cellgov_effects::WritePayload;
        use cellgov_event::PriorityClass;
        use cellgov_mem::{ByteRange, GuestAddr};
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= 2 {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        match n {
            1 => {
                // FIFO (RSX byte order, little-endian): OFFSET
                // header, offset arg, RELEASE header, release arg.
                let header_offset: u32 =
                    (1u32 << NV_COUNT_SHIFT) | (NV406E_SEMAPHORE_OFFSET as u32);
                let header_release: u32 =
                    (1u32 << NV_COUNT_SHIFT) | (NV406E_SEMAPHORE_RELEASE as u32);
                let words = [
                    header_offset,
                    self.sem_offset,
                    header_release,
                    self.release_value,
                ];
                let mut fifo_bytes: Vec<u8> = Vec::with_capacity(16);
                for w in words {
                    fifo_bytes.extend_from_slice(&w.to_le_bytes());
                }
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(self.fifo_base as u64), 16).unwrap(),
                    bytes: WritePayload::new(fifo_bytes),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
                // Advance put via the control-register mirror.
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(RSX_CONTROL_PUT_ADDR as u64), 4).unwrap(),
                    bytes: WritePayload::new(self.put_target.to_be_bytes().to_vec()),
                    ordering: PriorityClass::Normal,
                    source: self.id,
                    source_time: GuestTicks::ZERO,
                });
            }
            2 => {
                // No effects; commit_step drains the RsxLabelWrite queued in batch 1.
            }
            _ => {}
        }
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn build_with_rsx_and_label_region(label_base: u32) -> Runtime {
    use cellgov_mem::{GuestMemory, PageSize, Region};
    let regions = vec![
        Region::new(0, 0x10000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    rt.hle.gcm.label_addr = label_base;
    rt
}

#[test]
fn rsx_label_write_round_trip_drives_label_memory_end_to_end() {
    const FIFO_BASE: u32 = 0x200;
    const LABEL_BASE: u32 = 0x4000;
    const SEM_OFFSET: u32 = 0x10;
    const RELEASE_VALUE: u32 = 0xCAFE_BABE;
    let mut rt = build_with_rsx_and_label_region(LABEL_BASE);
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut()
        .register_with(|id| RsxOffsetReleaseDriverUnit {
            id,
            steps: Cell::new(0),
            fifo_base: FIFO_BASE,
            put_target: FIFO_BASE + 16,
            sem_offset: SEM_OFFSET,
            release_value: RELEASE_VALUE,
        });
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), FIFO_BASE + 16);
    assert_eq!(
        rt.rsx_cursor().get(),
        FIFO_BASE + 16,
        "drain must consume the full FIFO"
    );
    // Label memory untouched at end of step 1; RSX effects
    // commit in batch N+1 per the atomic-batch contract.
    use cellgov_mem::{ByteRange, GuestAddr};
    let pre_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new((LABEL_BASE + SEM_OFFSET) as u64), 4).unwrap())
        .unwrap();
    assert_eq!(pre_bytes, &[0, 0, 0, 0], "label still zero after batch 1");
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    let post_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new((LABEL_BASE + SEM_OFFSET) as u64), 4).unwrap())
        .unwrap();
    assert_eq!(
        post_bytes,
        &RELEASE_VALUE.to_be_bytes(),
        "label holds the big-endian release value after batch 2"
    );
}

#[test]
fn rsx_label_write_round_trip_is_deterministic_across_runs() {
    fn run() -> Vec<u8> {
        const FIFO_BASE: u32 = 0x200;
        const LABEL_BASE: u32 = 0x4000;
        let mut rt = build_with_rsx_and_label_region(LABEL_BASE);
        rt.set_rsx_mirror_writes(true);
        rt.registry_mut()
            .register_with(|id| RsxOffsetReleaseDriverUnit {
                id,
                steps: Cell::new(0),
                fifo_base: FIFO_BASE,
                put_target: FIFO_BASE + 16,
                sem_offset: 0x20,
                release_value: 0xDEAD_F00D,
            });
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
        }
        use cellgov_mem::{ByteRange, GuestAddr};
        rt.memory()
            .read(ByteRange::new(GuestAddr::new((LABEL_BASE + 0x20) as u64), 4).unwrap())
            .unwrap()
            .to_vec()
    }
    assert_eq!(run(), run());
}

#[test]
fn rsx_label_write_round_trip_same_final_state_at_two_budgets() {
    // Identical end state at Budget=1 vs Budget=16: commit-pipeline
    // path must not depend on how many instructions run per step.
    fn run_with_budget(budget: u64) -> Vec<u8> {
        use cellgov_mem::{GuestMemory, PageSize, Region};
        const FIFO_BASE: u32 = 0x200;
        const LABEL_BASE: u32 = 0x4000;
        let regions = vec![
            Region::new(0, 0x10000, "flat", PageSize::Page4K),
            Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
        ];
        let mem = GuestMemory::from_regions(regions).unwrap();
        let mut rt = Runtime::new(mem, Budget::new(budget), 100);
        rt.hle.gcm.label_addr = LABEL_BASE;
        rt.set_rsx_mirror_writes(true);
        rt.registry_mut()
            .register_with(|id| RsxOffsetReleaseDriverUnit {
                id,
                steps: Cell::new(0),
                fifo_base: FIFO_BASE,
                put_target: FIFO_BASE + 16,
                sem_offset: 0x30,
                release_value: 0x1234_5678,
            });
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
        }
        use cellgov_mem::{ByteRange, GuestAddr};
        rt.memory()
            .read(ByteRange::new(GuestAddr::new((LABEL_BASE + 0x30) as u64), 4).unwrap())
            .unwrap()
            .to_vec()
    }
    assert_eq!(run_with_budget(1), run_with_budget(16));
    assert_eq!(run_with_budget(1), 0x1234_5678u32.to_be_bytes().to_vec());
}

#[test]
fn rsx_mirror_writes_fires_fifo_advance_in_same_batch() {
    // Ordering contract: mirror runs before rsx_advance inside
    // commit_step. Empty zero-header FIFO means advance reaches
    // put cleanly; assertion is cursor.get == cursor.put post-commit.
    use crate::rsx::RSX_CONTROL_PUT_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_PUT_ADDR as u64,
        // 0x40 is inside the zero-initialised flat region;
        // zero-count headers decode as no-op method 0.
        value: 0x40,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), 0x40);
    assert_eq!(
        rt.rsx_cursor().get(),
        0x40,
        "advance pass must have drained after the mirror updated put"
    );
}

#[test]
fn sync_state_hash_changes_after_reservation_acquire() {
    let mut rt = build(4096, 1, 100);
    rt.registry_mut().register_with(|id| ReservationDriverUnit {
        id,
        steps: Cell::new(0),
        line_addr: 0x100,
    });
    let h0 = rt.sync_state_hash();
    // Step 1 emits ReservationAcquire.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let h1 = rt.sync_state_hash();
    assert_ne!(h0, h1, "reservation acquire must shift sync_state_hash");
}

#[test]
fn sync_state_hash_returns_to_empty_after_reservation_cleared() {
    let mut rt = build(4096, 1, 100);
    rt.registry_mut().register_with(|id| ReservationDriverUnit {
        id,
        steps: Cell::new(0),
        line_addr: 0x100,
    });
    let h_empty = rt.sync_state_hash();
    // Step 1: acquire.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_ne!(h_empty, rt.sync_state_hash());
    // Step 2: write to reserved line -> clear sweep drops the entry.
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    assert_eq!(
        h_empty,
        rt.sync_state_hash(),
        "cleared reservation table must restore the pre-acquire sync hash"
    );
}

#[test]
fn sync_state_hash_deterministic_across_identical_runs() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        rt.registry_mut().register_with(|id| ReservationDriverUnit {
            id,
            steps: Cell::new(0),
            line_addr: 0x100,
        });
        let mut hashes = vec![rt.sync_state_hash()];
        for _ in 0..2 {
            let s = rt.step().unwrap();
            rt.commit_step(&s.result, &s.effects).unwrap();
            hashes.push(rt.sync_state_hash());
        }
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_shifts_on_rsx_cursor_put_advance() {
    // Pins the RSX cursor's fold into sync_state_hash.
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_cursor_mut().set_put(0x20);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "rsx_cursor.put change must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_cursor_fields() {
    // Each of the cursor's three fields contributes independently.
    fn hash_with(put: u32, get: u32, reference: u32) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.rsx_cursor_mut().set_put(put);
        rt.rsx_cursor_mut().set_get(get);
        rt.rsx_cursor_mut().set_reference(reference);
        rt.sync_state_hash()
    }
    let base = hash_with(0, 0, 0);
    assert_ne!(base, hash_with(1, 0, 0), "put field must fold in");
    assert_ne!(base, hash_with(0, 1, 0), "get field must fold in");
    assert_ne!(
        base,
        hash_with(0, 0, 1),
        "current_reference field must fold in"
    );
}

#[test]
fn sync_state_hash_deterministic_across_rsx_mutation_sequence() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        let mut hashes = vec![rt.sync_state_hash()];
        rt.rsx_cursor_mut().set_put(0x20);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_get(0x10);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_reference(0x1234_5678);
        hashes.push(rt.sync_state_hash());
        rt.rsx_cursor_mut().set_put(0x40);
        hashes.push(rt.sync_state_hash());
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_shifts_on_rsx_flip_request() {
    // Pins the flip state's fold into sync_state_hash.
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_flip_mut().request_flip(0);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "flip request must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_flip_fields() {
    // Each flip-state field contributes independently.
    fn hash_with(status: u8, handler: u32, pending: bool, buffer_index: u8) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.rsx_flip_mut()
            .restore(status, handler, pending, buffer_index);
        rt.sync_state_hash()
    }
    let base = hash_with(0, 0, false, 0);
    assert_ne!(base, hash_with(1, 0, false, 0), "flip status folds in");
    assert_ne!(base, hash_with(0, 1, false, 0), "flip handler folds in");
    assert_ne!(base, hash_with(0, 0, true, 0), "flip pending folds in");
    assert_ne!(
        base,
        hash_with(0, 0, false, 1),
        "flip buffer_index folds in"
    );
}

#[test]
fn sync_state_hash_returns_to_empty_after_flip_completes() {
    let mut rt = build(4096, 1, 100);
    let h_empty = rt.sync_state_hash();
    rt.rsx_flip_mut().request_flip(0);
    assert_ne!(h_empty, rt.sync_state_hash());
    rt.rsx_flip_mut().complete_pending_flip();
    assert_eq!(
        h_empty,
        rt.sync_state_hash(),
        "DONE + pending=false + buffer_index=0 must equal the initial hash"
    );
}

#[test]
fn sync_state_hash_deterministic_across_rsx_flip_sequence() {
    fn run() -> Vec<u64> {
        let mut rt = build(4096, 1, 100);
        let mut hashes = vec![rt.sync_state_hash()];
        rt.rsx_flip_mut().set_handler(0x1000);
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().request_flip(1);
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().complete_pending_flip();
        hashes.push(rt.sync_state_hash());
        rt.rsx_flip_mut().request_flip(2);
        hashes.push(rt.sync_state_hash());
        hashes
    }
    assert_eq!(run(), run());
}

#[test]
fn sync_state_hash_distinguishes_different_reserved_lines() {
    fn run(line_addr: u64) -> u64 {
        let mut rt = build(4096, 1, 100);
        rt.registry_mut().register_with(|id| ReservationDriverUnit {
            id,
            steps: Cell::new(0),
            line_addr,
        });
        // Acquire without a subsequent write.
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        rt.sync_state_hash()
    }
    assert_ne!(
        run(0x100),
        run(0x200),
        "different reserved lines must hash differently"
    );
}

// -- Lost-reservation regression across write paths --

/// DMA completions apply via `fire_dma_completions` / direct
/// `apply_commit`, bypassing `SharedWriteIntent`. The clear
/// sweep must still fire on the destination so a cross-unit DMA
/// does not leave a stale reservation entry.
#[test]
fn dma_completion_clears_overlapping_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    // DmaPut requires a committed source range.
    {
        use cellgov_mem::{ByteRange, GuestAddr};
        let range = ByteRange::new(GuestAddr::new(0x80), 4).unwrap();
        rt.memory_mut().apply_commit(range, &[0x11; 4]).unwrap();
    }
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    // Trail the DMA with LoadImms so guest time advances past the 10-tick latency.
    let mut ops = vec![FakeOp::DmaPut {
        src: 0x80,
        dst: 0x0,
        len: 4,
    }];
    for _ in 0..30 {
        ops.push(FakeOp::LoadImm(0));
    }
    ops.push(FakeOp::End);
    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, ops));

    let mut completions_fired = 0usize;
    for _ in 0..100 {
        match rt.step() {
            Ok(step) => {
                let outcome = rt.commit_step(&step.result, &step.effects).unwrap();
                completions_fired += outcome.dma_completions_fired;
                if completions_fired > 0 && !rt.reservations().is_held_by(UnitId::new(1)) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        completions_fired > 0,
        "DMA completion must fire within the step budget"
    );
    assert!(
        !rt.reservations().is_held_by(UnitId::new(1)),
        "DMA completion to reserved line must clear unit 1's reservation"
    );
}

/// Runtime-level pin for the same-unit SharedWriteIntent clear
/// path; see commit_tests for the pipeline-level proof.
#[test]
fn plain_shared_write_through_runtime_clears_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0x42),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });

    for _ in 0..5 {
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result, &step.effects);
            }
            Err(_) => break,
        }
    }
    assert!(
        !rt.reservations().is_held_by(UnitId::new(1)),
        "plain SharedWriteIntent must clear cross-unit reservations"
    );
}

/// ConditionalStore clears both the emitter's own reservation
/// and overlapping entries; pins stwcx / putllc success
/// retirement across both paths.
#[test]
fn conditional_store_through_runtime_clears_own_and_overlapping() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(0), cellgov_sync::ReservedLine::containing(0));
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    // FakeIsaUnit lacks a local reservation register; this
    // exercises the commit-pipeline retirement path.
    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::ConditionalStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    // Unit 1 holds the stale reservation that must get cleared.
    rt.registry_mut()
        .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

    for _ in 0..5 {
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result, &step.effects);
            }
            Err(_) => break,
        }
    }
    assert!(!rt.reservations().is_held_by(UnitId::new(0)));
    assert!(!rt.reservations().is_held_by(UnitId::new(1)));
}

// -- Multi-primitive determinism canary with RSX content --

/// Emits `count` `RsxFlipRequest` effects one per step, cycling
/// buffer_index so each commit hash depends on emission order.
struct RsxFlipSpinnerUnit {
    id: UnitId,
    steps: Cell<u64>,
    count: u64,
}

impl ExecutionUnit for RsxFlipSpinnerUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= self.count {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n >= self.count {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::RsxFlipRequest {
            buffer_index: (n & 0x7) as u8,
        });
        ExecutionStepResult {
            yield_reason,
            consumed_budget: budget,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Four-unit canary: atomic contention + disjoint writes + 10
/// RSX flip cycles. sync_state_hash folds flip state, so any
/// RSX determinism drift surfaces as a final-hash mismatch.
#[test]
fn multi_primitive_determinism_canary_with_rsx_content() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> (u64, u64) {
        let mut rt = build(256, 4, 2000);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::LoadImm(0x22),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x33),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x77),
                    FakeOp::SharedStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| RsxFlipSpinnerUnit {
            id,
            steps: Cell::new(0),
            count: 10,
        });

        for _ in 0..500 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
        }
        (rt.memory().content_hash(), rt.sync_state_hash())
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "extended multi-primitive canary must produce byte-identical final (memory, sync) hashes across runs"
    );
}

/// Per-commit hash sequence must match across two runs; pins
/// every intermediate commit boundary, not just final state.
#[test]
fn multi_primitive_determinism_canary_rsx_per_step_hash_sequence_stable() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> Vec<u64> {
        let mut rt = build(256, 4, 2000);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| RsxFlipSpinnerUnit {
            id,
            steps: Cell::new(0),
            count: 5,
        });
        let mut hashes = vec![rt.sync_state_hash()];
        for _ in 0..500 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                    hashes.push(rt.sync_state_hash());
                }
                Err(_) => break,
            }
        }
        hashes
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "per-step sync_state_hash sequence must be byte-identical across two runs"
    );
    assert!(
        run_a.len() >= 8,
        "canary must have at least a handful of commits for a meaningful per-step comparison, got {}",
        run_a.len()
    );
}

// -- Multi-primitive determinism canary (atomic content) --

/// Two units contend for line 0 via acquire + conditional-store
/// cycles; a third emits disjoint shared writes. Combines
/// scheduler determinism with atomic-contention content.
#[test]
fn multi_primitive_determinism_canary_with_atomic_content() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> (u64, u64) {
        let mut rt = build(256, 4, 500);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::LoadImm(0x22),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x33),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        // Unit 2: disjoint shared writes (line 0x80).
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x77),
                    FakeOp::SharedStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });

        for _ in 0..200 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
        }
        (rt.memory().content_hash(), rt.sync_state_hash())
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "multi-primitive atomic canary must produce byte-identical final (memory, sync) hashes across runs"
    );
}

#[test]
#[should_panic(expected = "non-empty tls_bytes requires non-zero tls_base")]
fn ppu_thread_create_tls_base_zero_with_non_empty_tls_panics() {
    // Host rejects this with CELL_EINVAL in
    // `dispatch_ppu_thread_create`. A bad dispatch reaching the
    // apply path is a host bug, so the runtime asserts in release too.
    let mut rt = build(16, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let dispatch = cellgov_lv2::Lv2Dispatch::PpuThreadCreate {
        id_ptr: 0,
        init: cellgov_lv2::PpuThreadInitState {
            entry_code: 0,
            entry_toc: 0,
            arg: 0,
            stack_top: 0,
            tls_base: 0,
            lr_sentinel: 0,
        },
        stack_base: 0,
        stack_size: 0,
        tls_bytes: vec![0xAB, 0xCD],
        priority: 0,
        effects: vec![],
    };
    rt.handle_ppu_thread_create_for_test(source, dispatch);
}

#[test]
#[should_panic(expected = "unfilled payload")]
fn event_queue_receive_wake_with_none_payload_panics() {
    // Waking a receiver with no response_update must panic
    // rather than deliver four zero u64s as if they were real.
    let mut rt = build(16, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter,
        cellgov_lv2::PendingResponse::EventQueueReceive {
            out_ptr: 0x10,
            payload: None,
        },
    );
    rt.resolve_sync_wakes_for_test(&[waiter]);
}

// sys_rsx end-to-end microtests: each runs cellGcmInitBody with
// rsx_checkpoint off so HLE forwards to sys_rsx, exercising the
// full HLE + LV2 dispatch + commit path without external builds.

fn runtime_with_cellgcm_inited() -> (Runtime, cellgov_event::UnitId) {
    let mut rt = Runtime::new(
        cellgov_mem::GuestMemory::new(0x4000_0000),
        Budget::new(1),
        100,
    );
    rt.set_hle_heap_base(0x10_0000);
    rt.set_gcm_rsx_checkpoint(false);
    let unit_id = cellgov_event::UnitId::new(0);
    rt.registry_mut()
        .register_with(|id| cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End]));
    let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
    rt.dispatch_hle(
        unit_id,
        crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
        &args,
    );
    (rt, unit_id)
}

fn read_guest_u32_be(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

fn read_guest_u64_be(rt: &Runtime, addr: u32) -> u64 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u64::from_be_bytes([
        mem[a],
        mem[a + 1],
        mem[a + 2],
        mem[a + 3],
        mem[a + 4],
        mem[a + 5],
        mem[a + 6],
        mem[a + 7],
    ])
}

#[test]
fn rsx_context_allocate_init_pattern() {
    let (rt, _) = runtime_with_cellgcm_inited();
    let reports_base = rt.lv2_host().sys_rsx_context().reports_addr;

    // Semaphore sentinels at the first group-of-4.
    assert_eq!(read_guest_u32_be(&rt, reports_base), 0x1337_C0D3);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 4), 0x1337_BABE);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 8), 0x1337_BEEF);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 12), 0x1337_F001);

    // Notify[0] at +0x1000: timestamp=-1, zero=0.
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1000), u64::MAX);
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1008), 0);

    // Report[0] at +0x1400: timestamp=-1, val=0, pad=-1.
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1400), u64::MAX);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 0x1408), 0);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 0x140C), u32::MAX);
}

#[test]
fn rsx_label_255_sentinel_read() {
    // cellGcmGetLabelAddress(255) must read the LV2 sentinel
    // 0x1337_C0D3; addr math is label_addr + 255 * 0x10.
    let (rt, _) = runtime_with_cellgcm_inited();
    let label_addr = rt.hle.gcm.label_addr;
    let label_255_addr = label_addr + 255 * 0x10;
    assert_eq!(read_guest_u32_be(&rt, label_255_addr), 0x1337_C0D3);
}

#[test]
fn rsx_reports_region_full_size() {
    // Notify and report arrays carry init values past the 4 KB
    // semaphore region; a 4 KB-label-only layout would read zeros.
    let (rt, _) = runtime_with_cellgcm_inited();
    let label_addr = rt.hle.gcm.label_addr;
    let notify_63 = label_addr + 0x1000 + 63 * 16;
    assert_eq!(read_guest_u64_be(&rt, notify_63), u64::MAX);
    let report_2047_pad = label_addr + 0x1400 + 2047 * 16 + 12;
    assert_eq!(read_guest_u32_be(&rt, report_2047_pad), u32::MAX);
}

#[test]
fn rsx_dma_control_layout() {
    // put / get / ref at dma_control_addr + 0x40 / 0x44 / 0x48;
    // cellGcmGetControlRegister returns dma_control_addr + 0x40.
    let (rt, _) = runtime_with_cellgcm_inited();
    let dma_base = rt.lv2_host().sys_rsx_context().dma_control_addr;
    let ctrl = rt.hle.gcm.control_addr;
    assert_eq!(ctrl, dma_base + 0x40);
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x40), 0);
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x44), 0);
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x48), 0);
}

#[test]
fn rsx_event_port_registered() {
    // RsxDriverInfo.handler_queue at +0x12D0 holds the event-queue
    // id that sys_rsx_context_allocate created.
    let (rt, _) = runtime_with_cellgcm_inited();
    let driver_info_addr = rt.lv2_host().sys_rsx_context().driver_info_addr;
    let handler_queue = read_guest_u32_be(&rt, driver_info_addr + 0x12D0);
    assert_ne!(handler_queue, 0);
    assert_eq!(
        handler_queue,
        rt.lv2_host().sys_rsx_context().event_queue_id
    );
    assert_eq!(handler_queue, rt.lv2_host().sys_rsx_context().event_port_id);
}

#[test]
fn sys_rsx_dispatch_commutes_with_unrelated_unit_steps() {
    // Schedule-stability: sys_rsx dispatch writes a fixed set of
    // guest addresses, so with disjoint concurrent PPU writes,
    // final memory is independent of when sys_rsx fires.
    fn run_with_dispatch_at(position: usize) -> u64 {
        let mut rt = Runtime::new(
            cellgov_mem::GuestMemory::new(0x4000_0000),
            Budget::new(1),
            100,
        );
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(false);

        // PPU writers target addresses disjoint from the sys_rsx
        // region (which starts at 0x3000_0000).
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(
                id,
                vec![
                    cellgov_exec::FakeOp::LoadImm(0xAA),
                    cellgov_exec::FakeOp::SharedStore {
                        addr: 0x2_0000,
                        len: 4,
                    },
                    cellgov_exec::FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(
                id,
                vec![
                    cellgov_exec::FakeOp::LoadImm(0xBB),
                    cellgov_exec::FakeOp::SharedStore {
                        addr: 0x2_0100,
                        len: 4,
                    },
                    cellgov_exec::FakeOp::End,
                ],
            )
        });
        // sys_rsx needs a dispatch source.
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let sys_rsx_source = cellgov_event::UnitId::new(2);

        let sys_rsx_init = |rt: &mut Runtime| {
            let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
            rt.dispatch_hle(
                sys_rsx_source,
                crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
                &args,
            );
        };

        let mut step_count = 0;
        loop {
            if step_count == position {
                sys_rsx_init(&mut rt);
            }
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
            step_count += 1;
            if step_count > 20 {
                break;
            }
        }
        if step_count <= position {
            sys_rsx_init(&mut rt);
        }
        rt.memory().content_hash()
    }

    let early = run_with_dispatch_at(0);
    let mid = run_with_dispatch_at(2);
    let late = run_with_dispatch_at(10);

    assert_eq!(
        early, mid,
        "sys_rsx dispatch at step 0 vs step 2 must produce identical memory state"
    );
    assert_eq!(
        mid, late,
        "sys_rsx dispatch at step 2 vs step 10 must produce identical memory state"
    );
}

#[test]
fn multi_primitive_determinism_canary_with_sys_rsx_content() {
    // sys_rsx extension of the multi-primitive canary. If any
    // sys_rsx output path drifts between runs, the final
    // (memory, sync) hashes diverge.
    fn run_once() -> (u64, u64) {
        let mut rt = Runtime::new(
            cellgov_mem::GuestMemory::new(0x4000_0000),
            Budget::new(1),
            100,
        );
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(false);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
            &args,
        );
        // Sub-command flip path drives rsx_flip fold-in.
        let ctx_id = rt.lv2_host().sys_rsx_context().context_id;
        rt.dispatch_lv2_request(
            cellgov_lv2::Lv2Request::SysRsxContextAttribute {
                context_id: ctx_id,
                package_id: cellgov_lv2::host::PACKAGE_FLIP_BUFFER,
                a3: 0,
                a4: 0x8000_0000,
                a5: 0,
                a6: 0,
            },
            unit_id,
        );
        // Handler registration via sys_rsx.
        rt.dispatch_lv2_request(
            cellgov_lv2::Lv2Request::SysRsxContextAttribute {
                context_id: ctx_id,
                package_id: cellgov_lv2::host::PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0x1234_5678,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            unit_id,
        );
        (rt.memory().content_hash(), rt.sync_state_hash())
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "sys_rsx canary: memory_hash and sync_state_hash must \
         match byte-identically across two runs"
    );
}

#[test]
fn rsx_context_attribute_flip_drives_status_transitions() {
    // FLIP_BUFFER via sub-command 0x102 emits RsxFlipRequest;
    // one commit boundary later it transitions to DONE, matching
    // the NV4097_FLIP_BUFFER path.
    let (mut rt, unit_id) = runtime_with_cellgcm_inited();
    let ctx_id = rt.lv2_host().sys_rsx_context().context_id;

    assert_eq!(
        rt.rsx_flip().status(),
        crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "pre-request: DONE"
    );

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SysRsxContextAttribute {
            context_id: ctx_id,
            package_id: cellgov_lv2::host::PACKAGE_FLIP_BUFFER,
            a3: 0,
            a4: 0x8000_0001,
            a5: 0,
            a6: 0,
        },
        unit_id,
    );

    assert_eq!(
        rt.rsx_flip().status(),
        crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
        "post-request: WAITING + pending"
    );
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);

    // One empty commit boundary completes the two-batch WAITING -> DONE transition.
    rt.registry_mut()
        .register_with(|id| cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End]));
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.rsx_flip().status(),
        crate::rsx::flip::CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "post-boundary: DONE"
    );
    assert!(!rt.rsx_flip().pending());
}
