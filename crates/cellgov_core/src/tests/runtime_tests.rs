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
    rt.commit_step(&s1.result, &s1.effects).unwrap();
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
    rt.commit_step(&s.result, &s.effects).unwrap();
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
    rt.commit_step(&s.result, &s.effects).unwrap();
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
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
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
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
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
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.done.set(true);
            effects.push(Effect::SharedWriteIntent {
                // Range starts past end of memory -- definitely OOB.
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
    // First step: writes [1,1,1,1] to addr 0.
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
    // Epoch advanced by exactly one.
    assert_eq!(rt.epoch(), Epoch::new(1));

    // Second step: overwrites with [2,2,2,2].
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

    // FaultDriven should produce zero trace records.
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
    // Default is FullTrace.
    assert_eq!(rt.mode(), RuntimeMode::FullTrace);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    let reader = TraceReader::new(rt.trace().bytes());
    let records: Vec<_> = reader.collect();
    // FullTrace should emit at least: UnitScheduled, StepCompleted,
    // CommitApplied, and 4 hash checkpoints = 7 minimum.
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

/// Test unit that simulates per-step state-hash production. The
/// runtime should drain the configured pairs after run_until_yield
/// and emit one TraceRecord::PpuStateHash per pair, in order, with
/// monotonically incrementing step indices that span across multiple
/// run_until_yield calls.
type FullStateTuple = (u64, [u64; 32], u64, u64, u64, u32);

struct StateHashEmittingUnit {
    id: UnitId,
    pairs_per_step: Vec<Vec<(u64, u64)>>,
    /// Optional 9G zoom-in snapshots, paired with `pairs_per_step`.
    /// Inner Vec empty means no full-state records that step.
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
        // First call retires 2 instructions; second retires 1; third retires 0.
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
    let _ = StateHash::new(0); // touch to keep the import in use even if hashes are filtered
}

#[test]
fn runtime_emits_no_ppu_state_hash_when_unit_drains_empty() {
    use cellgov_trace::{TraceReader, TraceRecord};
    // CountingUnit does not override drain_retired_state_hashes, so
    // it returns the trait default (empty). Runtime must emit zero
    // PpuStateHash records regardless of how many steps run.
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
        // Three retired instructions, of which only the middle one
        // is inside a hypothetical zoom-in window.
        pairs_per_step: vec![vec![(0x100, 0xaa), (0x104, 0xbb), (0x108, 0xcc)]],
        full_per_step: vec![vec![(0x104, [0u64; 32], 0, 0, 0, 0)]],
        step_idx: Cell::new(0),
    });
    rt.step().unwrap();

    // Main trace: 3 PpuStateHash, 0 PpuStateFull (the main stream is
    // homogeneous; full states never get mixed in).
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

    // Zoom trace: 1 PpuStateFull at the windowed PC.
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

// ---------------------------------------------------------------
// Trivial-commit fast path -- three proof microtests.
//
// The trivial-step fast path must not alter the observable
// contract in any of the following scenarios.
// ---------------------------------------------------------------

/// Unit that emits zero effects every step and finishes after
/// `max` steps. Used by the empty-loop gate below -- its steps
/// are exactly the kind the trivial-commit fast path short-
/// circuits.
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

/// Trivial-commit proof 1 -- empty loop.
///
/// 10K non-effect-emitting steps under FaultDriven mode must
/// advance the epoch exactly 10K times (one per step, matching
/// the atomic-batch boundary contract) and emit zero trace
/// records (FaultDriven mode's existing guarantee). The fast
/// path must not break either.
#[test]
fn commit_fast_path_empty_loop_advances_epoch_monotonically() {
    let mut rt = build(64, 1, 20_000);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Finished yields take the slow path; BudgetExhausted does
    // not. Give the unit enough runway so all 10K steps are
    // BudgetExhausted.
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
    // FaultDriven mode already suppresses trace records; the
    // fast path must preserve that guarantee on empty-effect
    // steps. Empty trace bytes is the strongest assertion.
    assert!(
        rt.trace().bytes().is_empty(),
        "FaultDriven + empty-effect steps must produce no trace records"
    );
}

/// Trivial-commit proof 2 -- DMA completion near a fast-path stretch.
///
/// A DMA is enqueued with a fixed completion tick; the issuing
/// unit emits nothing afterwards (silent steps). The completion
/// must fire at the step whose accumulated time crosses the
/// scheduled tick, not one step earlier or later, and the
/// transfer must apply to committed memory at that boundary.
///
/// The fast path checks `dma_queue.is_empty()` and takes the
/// slow path as soon as a DMA is pending, so this test also
/// exercises the "enter slow path when async work is present"
/// transition.
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
    // Scheduled at tick 3. Budget=1 per step, so step 3 is the
    // first where accumulated time reaches 3.
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100,
    });

    // Step 1: time -> 1. DMA pending, slow path. Not yet due.
    let s = rt.step().unwrap();
    let o1 = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o1.dma_completions_fired, 0);
    // Step 2: time -> 2. Still pending.
    let s = rt.step().unwrap();
    let o2 = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o2.dma_completions_fired, 0);
    // Step 3: time -> 3. DMA fires at its scheduled tick, and
    // the transfer applies to memory in the same commit.
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
    // Step 4+: queue empty again. Fast path re-engages; epoch
    // still advances.
    let epoch_before = rt.epoch();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.epoch().raw(), epoch_before.raw() + 1);
}

/// Trivial-commit proof 3 -- wake visibility through a silent stretch.
///
/// Unit A, while blocked, is woken by a DMA completion fired on
/// unit B's step. That wake is observed outside the commit path
/// (via `registry().effective_status`) immediately after the
/// firing step and must stay visible through subsequent silent
/// (fast-path) steps -- the status flip is carried by the
/// `status_overrides` map, which the fast path does not touch.
#[test]
fn commit_fast_path_preserves_wake_visibility_through_silent_steps() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};

    let mut rt = build(256, 1, 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Unit 0 is the DMA-issuing waiter; it starts Blocked so the
    // scheduler never picks it until the completion fires.
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

    // Step 1: unit 1 runs (unit 0 Blocked), time -> 1. Slow path
    // (DMA pending); nothing fires yet.
    let s = rt.step().unwrap();
    assert_eq!(s.unit, UnitId::new(1));
    let o = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(o.dma_completions_fired, 0);
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Blocked)
    );
    // Step 2: unit 1 runs again, time -> 2, DMA fires, unit 0
    // wakes. The wake is a status_override flip, observable
    // immediately.
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
    // Step 3: unit 0 is now Runnable; the scheduler will pick
    // whichever is first. We force silent steps on both units
    // and verify the wake visibility is preserved.
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    // Unit 0's override is cleared the first time it runs (see
    // `clear_status_override` in step); but the wake happened
    // at `wake_epoch`, and the epoch has advanced exactly once
    // per subsequent commit regardless of whether the fast or
    // slow path ran.
    assert_eq!(
        rt.epoch().raw(),
        wake_epoch.raw() + 1,
        "epoch must advance once per commit, fast or slow"
    );
}

// --- Reservation state folds into sync_state_hash ---

/// A unit that emits a single `ReservationAcquire` effect on its
/// first step and a single `SharedWriteIntent` to the reserved
/// line on its second step, then finishes. Used to exercise the
/// sync_state_hash folding with a scripted acquire / release
/// sequence.
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
                // Overlapping write clears the reservation.
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

/// Writes FIFO bytes encoding a single `GCM_FLIP_COMMAND` method
/// with the given buffer index, plus advances put via the RSX
/// control-register mirror. Finishes after one step.
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
        // FIFO words: GCM_FLIP_COMMAND header (count=1) + arg.
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
    // End-to-end flip state machine:
    // Batch 1: unit emits FLIP_BUFFER + put advance. FIFO drain
    //   parses and queues RsxFlipRequest into pending.
    //   Flip state at end of batch 1: DONE (unchanged) -- the
    //   RsxFlipRequest has NOT been applied yet (lands in batch 2).
    // Batch 2: commit pipeline applies the queued RsxFlipRequest,
    //   flipping status to WAITING / pending=true. pending_at_entry
    //   was false (from batch 1), so no DONE transition.
    //   Flip state at end of batch 2: WAITING / pending=true.
    // Batch 3 (or any later boundary): pending_at_entry=true,
    //   DONE transition fires.
    //   Flip state at end of batch 3: DONE / pending=false.
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
    // Batch 1: emits FIFO + put; drain parses FLIP_BUFFER, queues effect.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 1 end: effect queued, not yet applied; flip still DONE"
    );
    assert!(!rt.rsx_flip().pending());
    // Batch 2: the unit is finished, no new unit emissions. Commit
    // drains pending_rsx_effects (our queued RsxFlipRequest). Need
    // a step -- but the unit is Finished, which means rt.step()
    // would return NoRunnableUnit. Use a trivial second unit.
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
    // Batch 3: pending_at_entry=true, DONE transition fires.
    let s3 = rt.step().unwrap();
    rt.commit_step(&s3.result, &s3.effects).unwrap();
    assert_eq!(
        rt.rsx_flip().status(),
        CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
        "batch 3 end: transition fired"
    );
    assert!(!rt.rsx_flip().pending());
}

/// Unit that emits a single `RsxFlipRequest` effect on its first
/// step, then finishes. Used to exercise the commit pipeline's
/// RsxFlipRequest application path without going through the FIFO
/// drain (which adds a one-batch delay).
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
    // Tightness probe: when a RsxFlipRequest is applied in this
    // commit, the DONE transition must NOT fire in the same
    // batch. `flip_pending_at_entry` was false (pending hasn't
    // been set yet), so the post-apply hook skips the transition.
    // Pins the "one-batch-delay" contract the design doc calls out.
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
    // Companion to the tightness probe: after the WAITING state
    // has been observable for one PPU step, the NEXT commit
    // boundary transitions to DONE. Two-step test: step 1 emits
    // RsxFlipRequest (WAITING + pending=true); step 2 is any
    // commit (no flip-related emission); DONE transition fires
    // because pending_at_entry was true.
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
    // Corner case: a second RsxFlipRequest that arrives while
    // pending==true updates buffer_index but does
    // NOT add a second transition -- exactly one WAITING -> DONE
    // fires for the sequence.
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
    // Batch 1: first RsxFlipRequest (buffer 1) -> WAITING + pending.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert!(rt.rsx_flip().pending());
    assert_eq!(rt.rsx_flip().buffer_index(), 1);
    // Batch 2: second RsxFlipRequest (buffer 5) overwrites
    // buffer_index BUT pending_at_entry was true so DONE fires.
    // Post-apply: RsxFlipRequest runs first (WAITING, pending,
    // buffer=5), then DONE transition checks pending_at_entry=true
    // and fires. So end state: DONE + pending=false + buffer=5.
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

/// Writes a u32 to a specific RSX control-register slot via a
/// single SharedWriteIntent on its first step, then finishes. The
/// slot address and value are constructor params so a test can
/// target put / get / reference individually.
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
    // Memory also holds the big-endian value (byte-for-byte what
    // the guest wrote) so a guest read-back sees its own store.
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

/// Drives a full RSX label-write round trip inside the runtime:
/// Step 1 writes an OFFSET + RELEASE method pair to FIFO memory
/// via SharedWriteIntents AND advances put via the RSX control
/// register mirror. Step 2 (any subsequent commit) drains the
/// pending RsxLabelWrite emitted by the FIFO advance pass of step 1
/// into memory at `label_base + offset`. The test asserts the
/// final memory byte is the expected value.
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
                // Step 1: write FIFO words and advance put. Four
                // FIFO words: OFFSET header, offset arg, RELEASE
                // header, release arg. All little-endian (RSX
                // byte order).
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
                // Step 2: no effects -- commit_step must drain
                // the pending RsxLabelWrite emitted at the end of
                // batch 1's FIFO advance pass into memory.
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
        // Flat region for FIFO.
        Region::new(0, 0x10000, "flat", PageSize::Page4K),
        // RSX region for control-register writes.
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    rt.hle.gcm.label_addr = label_base;
    rt
}

#[test]
fn rsx_label_write_round_trip_drives_label_memory_end_to_end() {
    // FIFO at 0x200. sem_offset = 0x10. label_base = 0x4000. The
    // final label write lands at 0x4010 as big-endian 0xCAFE_BABE.
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
    // Step 1: emits FIFO bytes + put advance; triggers drain; queues RsxLabelWrite.
    let s1 = rt.step().unwrap();
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    assert_eq!(rt.rsx_cursor().put(), FIFO_BASE + 16);
    assert_eq!(
        rt.rsx_cursor().get(),
        FIFO_BASE + 16,
        "drain must consume the full FIFO"
    );
    // Label memory still untouched at end of step 1 -- RSX effects
    // commit in batch N+1 per the atomic-batch contract.
    use cellgov_mem::{ByteRange, GuestAddr};
    let pre_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new((LABEL_BASE + SEM_OFFSET) as u64), 4).unwrap())
        .unwrap();
    assert_eq!(pre_bytes, &[0, 0, 0, 0], "label still zero after batch 1");
    // Step 2: no unit effects; commit drains pending RsxLabelWrite.
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
    // Same input -> byte-identical final label memory across two
    // independent runs. Closes the "same-budget replay
    // determinism" gate for the RSX label-write flow.
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
    // Cross-budget final-state equivalence: identical end state
    // after running with Budget=1 vs Budget=16. Both scenarios
    // complete the same scripted work; the commit-pipeline path
    // must not depend on how many instructions the unit consumes
    // per step.
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
    // Integration: a single SharedWriteIntent that advances put
    // must feed the cursor AND trigger the FIFO advance pass in
    // the same commit_step. This pins the ordering contract
    // (mirror runs BEFORE rsx_advance in commit_step). Use an
    // empty FIFO (no methods between get=0 and put=0x10 because
    // memory is zero-initialised; zero-headers are NOPs). Assert
    // cursor.get advanced to cursor.put after commit.
    use crate::rsx::RSX_CONTROL_PUT_ADDR;
    let mut rt = build_with_rsx_writable();
    rt.set_rsx_mirror_writes(true);
    rt.registry_mut().register_with(|id| RsxControlWriterUnit {
        id,
        steps: Cell::new(0),
        slot_addr: RSX_CONTROL_PUT_ADDR as u64,
        // Point put into the flat region (address 0x40 is within
        // the zero-initialised base-0 region). Zero-count headers
        // decode as Increment method=0 count=0 which are
        // unknown-method no-ops; drain reaches put cleanly.
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
    // Advancing the RSX cursor's put pointer must change
    // sync_state_hash even with no units registered and no memory
    // writes. Pins the cursor's fold into the sync hash so a future
    // refactor that drops the fold breaks this test immediately.
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_cursor_mut().set_put(0x20);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "rsx_cursor.put change must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_cursor_fields() {
    // Each of the cursor's three fields must contribute to the
    // hash independently. Catches a fold that only mixes one field
    // or masks one out.
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
    // Scripted cursor-mutation sequence must produce a byte-
    // identical per-step sync_state_hash sequence across two
    // runs. This is the per-slice determinism check for the
    // state-hash folding.
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
    // A flip request from DONE to WAITING must change
    // sync_state_hash. Pins the flip state's fold into the sync
    // hash so a future refactor that drops the fold breaks this
    // test immediately.
    let rt_a = build(4096, 1, 100);
    let mut rt_b = build(4096, 1, 100);
    let h_a = rt_a.sync_state_hash();
    rt_b.rsx_flip_mut().request_flip(0);
    let h_b = rt_b.sync_state_hash();
    assert_ne!(h_a, h_b, "flip request must shift sync_state_hash");
}

#[test]
fn sync_state_hash_distinguishes_flip_fields() {
    // Each flip-state field contributes to the hash independently.
    // Catches a fold that only mixes one field or masks one out.
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
    // Scripted round trip: request + complete must restore the
    // pre-request sync_state_hash. Pending flips to false, status
    // returns to DONE, handler and buffer_index are unchanged
    // from the baseline (both zero).
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
    // Scripted flip sequence produces byte-identical per-step
    // sync_state_hash sequences across two runs. Pins the
    // flip-state determinism gate.
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
        // Step 1 only: acquire but do not write.
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

// --- Lost-reservation regression across write paths ---

/// DMA completions bypass `SharedWriteIntent` -- they are applied
/// by `fire_dma_completions` via a direct `apply_commit` path.
/// The clear sweep must still fire on the destination range so a
/// cross-unit DMA does not leave a stale reservation entry that
/// a later stwcx / putllc would spuriously read as "still held."
///
/// Test: unit 1 pre-holds a reservation on line 0x0. Unit 0
/// emits a DmaPut whose destination covers the line. After
/// scheduling runs through the DMA's latency (10 ticks), the
/// completion fires and the reservation must be cleared.
#[test]
fn dma_completion_clears_overlapping_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    // DmaPut wants a committed source range. Pre-populate one.
    {
        use cellgov_mem::{ByteRange, GuestAddr};
        let range = ByteRange::new(GuestAddr::new(0x80), 4).unwrap();
        rt.memory_mut().apply_commit(range, &[0x11; 4]).unwrap();
    }
    // Pre-populate unit 1's reservation on line 0x0.
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    // Unit 0: emit DmaPut 0x80 -> 0x0, 4 bytes, then run many
    // LoadImms so guest time keeps advancing past the DMA's
    // 10-tick latency. Finally End.
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

/// Same-unit PPU store through the staging path is proven at the
/// commit-pipeline level in commit_tests::shared_write_clears_
/// reservation_covering_line. This runtime-level test pins the
/// end-to-end invariant by driving a SharedWriteIntent through
/// the full step / commit cycle and asserting the table clears.
#[test]
fn plain_shared_write_through_runtime_clears_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    // Unit 1 holds a reservation on line 0.
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    // Unit 0: plain shared store to line 0.
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

/// ConditionalStore path drives the clear sweep against both the
/// emitter's own reservation and any overlapping entries. Pins
/// that a real stwcx / putllc success retires both.
#[test]
fn conditional_store_through_runtime_clears_own_and_overlapping() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(0), cellgov_sync::ReservedLine::containing(0));
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

    // Unit 0 emits a ConditionalStore to line 0. (FakeIsaUnit
    // lacks a local reservation register; the test exercises the
    // commit-pipeline retirement path.)
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
    // Unit 1 does nothing (it only holds a stale reservation we want to see cleared).
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

// --- Multi-primitive determinism canary with RSX content ---

/// Unit that emits `count` `RsxFlipRequest` effects, one per step,
/// then finishes. Cycles the buffer_index across requests so each
/// commit hash depends on the exact emission order.
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

/// Extended multi-primitive canary. The three-unit atomic +
/// disjoint-write canary gets a fourth unit that drives the RSX
/// flip-status state machine through 10 WAITING -> DONE cycles.
/// Final (memory, sync) hashes -- sync_state_hash folds the flip
/// state, so any RSX determinism regression surfaces as a hash
/// mismatch -- must be byte-identical across two independent runs.
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

/// Per-commit sync-state-hash sequence must match across two runs
/// of the extended canary. Strongest form of the determinism
/// check: pins NOT just the final state but every intermediate
/// commit boundary.
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

// --- Multi-primitive determinism canary (atomic content) ---

/// Two units drive acquire / store / clear cycles against a
/// shared line; a third unit independently emits disjoint shared
/// writes. Running the full scenario twice must produce
/// byte-identical final memory AND byte-identical final sync-
/// state hashes. Combines the multi-unit scheduler-determinism
/// canary with atomic-contention content so a regression on
/// either axis fails loudly.
#[test]
fn multi_primitive_determinism_canary_with_atomic_content() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> (u64, u64) {
        let mut rt = build(256, 4, 500);
        // Unit 0 / 1: repeated acquire + conditional-store on
        // line 0.
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
    // Host rejects this combination with CELL_EINVAL in
    // `dispatch_ppu_thread_create`; a bad dispatch reaching the
    // runtime apply path is a host regression, so the runtime
    // asserts unconditionally (both debug and release) rather
    // than silently committing the TLS image to guest address 0.
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
    // The release side forgot to ship a response_update before
    // waking the receiver. The wake path must panic rather than
    // deliver four zero u64s the guest cannot distinguish from
    // a real event.
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

// sys_rsx end-to-end microtests. Each runs cellGcmInitBody with
// rsx_checkpoint off so the HLE forwards to sys_rsx, then asserts
// observable post-conditions (reports init pattern, label-255
// sentinel, reports region size, DMA control layout, event-port
// registration, flip sub-command state transitions). Exercised at
// the runtime layer rather than as guest ELFs so the full HLE +
// LV2 dispatch + commit pipeline path runs without external build
// dependencies.

fn phase21_runtime_with_cellgcm_inited() -> (Runtime, cellgov_event::UnitId) {
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
fn phase21_rsx_context_allocate_init_pattern() {
    // sys_rsx_context_allocate seeds the semaphore slots with the
    // repeating 4-u32 pattern (0x1337C0D3 / BABE / BEEF / F001),
    // notify timestamps with -1, and report timestamp + pad with -1.
    let (rt, _) = phase21_runtime_with_cellgcm_inited();
    let reports_base = rt.lv2_host().sys_rsx_context().reports_addr;

    // Semaphore sentinels at the first group-of-4.
    assert_eq!(read_guest_u32_be(&rt, reports_base), 0x1337_C0D3);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 4), 0x1337_BABE);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 8), 0x1337_BEEF);
    assert_eq!(read_guest_u32_be(&rt, reports_base + 12), 0x1337_F001);

    // Notify[0].timestamp at reports_base + 0x1000 = u64::MAX.
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1000), u64::MAX);
    // Notify[0].zero at +0x1008 = 0.
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1008), 0);

    // Report[0].timestamp at reports_base + 0x1400 = u64::MAX.
    assert_eq!(read_guest_u64_be(&rt, reports_base + 0x1400), u64::MAX);
    // Report[0].val at +0x1408 = 0.
    assert_eq!(read_guest_u32_be(&rt, reports_base + 0x1408), 0);
    // Report[0].pad at +0x140C = u32::MAX.
    assert_eq!(read_guest_u32_be(&rt, reports_base + 0x140C), u32::MAX);
}

#[test]
fn phase21_rsx_label_255_sentinel_read() {
    // cellGcmGetLabelAddress(255) returns the guest address that
    // reads the LV2 sentinel 0x1337_C0D3 post-init. Exercise
    // `label_addr + 255 * 0x10` directly since the label-address
    // math mirrors what cellGcmGetLabelAddress computes.
    let (rt, _) = phase21_runtime_with_cellgcm_inited();
    let label_addr = rt.hle.gcm.label_addr;
    let label_255_addr = label_addr + 255 * 0x10;
    assert_eq!(read_guest_u32_be(&rt, label_255_addr), 0x1337_C0D3);
}

#[test]
fn phase21_rsx_reports_region_full_size() {
    // Past the 4 KB semaphore region the notify and report arrays
    // carry their init values; the pre-sys_rsx 4 KB-label-region
    // world would have returned zeros here.
    let (rt, _) = phase21_runtime_with_cellgcm_inited();
    let label_addr = rt.hle.gcm.label_addr;
    // Notify[63] (last entry) timestamp sits at 0x1000 + 63 * 16.
    let notify_63 = label_addr + 0x1000 + 63 * 16;
    assert_eq!(read_guest_u64_be(&rt, notify_63), u64::MAX);
    // Report[2047] pad at end of region.
    let report_2047_pad = label_addr + 0x1400 + 2047 * 16 + 12;
    assert_eq!(read_guest_u32_be(&rt, report_2047_pad), u32::MAX);
}

#[test]
fn phase21_rsx_dma_control_layout() {
    // put / get / ref live at +0x40 / +0x44 / +0x48 from
    // dma_control_addr; cellGcmGetControlRegister returns
    // dma_control_addr + 0x40 as the guest-facing handle.
    let (rt, _) = phase21_runtime_with_cellgcm_inited();
    let dma_base = rt.lv2_host().sys_rsx_context().dma_control_addr;
    let ctrl = rt.hle.gcm.control_addr;
    assert_eq!(ctrl, dma_base + 0x40);
    // The three slots are zero-initialised by the HLE heap; this
    // test pins the layout, not the values.
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x40), 0);
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x44), 0);
    assert_eq!(read_guest_u32_be(&rt, dma_base + 0x48), 0);
}

#[test]
fn phase21_rsx_event_port_registered() {
    // RsxDriverInfo.handler_queue at offset 0x12D0 holds the
    // event-queue id sys_rsx_context_allocate created. Non-zero
    // means the queue exists; the LV2 host tracks the same id.
    let (rt, _) = phase21_runtime_with_cellgcm_inited();
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
fn phase21_sys_rsx_dispatch_commutes_with_unrelated_unit_steps() {
    // Schedule-stability check: sys_rsx syscall dispatch writes a
    // fixed set of guest addresses (reports / driver info / dma
    // control). If the concurrently-running PPU units touch only
    // disjoint addresses, the final memory state does not depend
    // on when during the run sys_rsx fires. This is the sys_rsx
    // analogue of the exploration engine's schedule-stable
    // classification.
    fn run_with_dispatch_at(position: usize) -> u64 {
        let mut rt = Runtime::new(
            cellgov_mem::GuestMemory::new(0x4000_0000),
            Budget::new(1),
            100,
        );
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(false);

        // Two FakeIsaUnits writing disjoint addresses well outside
        // the sys_rsx region (which starts at 0x3000_0000).
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
        // A third unit so sys_rsx has a dispatch source.
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
fn phase21_multi_primitive_determinism_canary_with_sys_rsx_content() {
    // sys_rsx extension of the multi-primitive determinism canary.
    // Both runs must produce byte-identical (memory, sync) hashes
    // after cellGcmInitBody forwards through sys_rsx (37 KB reports
    // init + 0x12F8 driver info init + 0x300000 reservation +
    // event-queue creation). If any sys_rsx output path drifts
    // between runs, these hashes diverge.
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
        // Drive the sub-command flip path too so rsx_flip contributes.
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
        // Handler registration routed through sys_rsx.
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
fn phase21_rsx_context_attribute_flip_drives_status_transitions() {
    // Dispatching FLIP_BUFFER via sub-command 0x102 emits an
    // RsxFlipRequest effect; one commit boundary later the flip
    // status transitions to DONE, matching the NV4097_FLIP_BUFFER
    // path.
    let (mut rt, unit_id) = phase21_runtime_with_cellgcm_inited();
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

    // Drive one empty commit boundary to advance the pending flip
    // to DONE, matching the NV4097_FLIP_BUFFER path's observable
    // two-batch transition.
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
