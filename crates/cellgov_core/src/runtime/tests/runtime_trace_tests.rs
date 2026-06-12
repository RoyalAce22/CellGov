//! Trace-record emission, level filtering, fault-discard traces, and zoom routing.

use super::*;

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
            consumed_cost,
            time_after,
        } => {
            assert_eq!(unit, UnitId::new(0));
            assert_eq!(yield_reason, TracedYieldReason::BudgetExhausted);
            assert_eq!(consumed_cost, InstructionCost::new(5));
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

    #[derive(Clone)]

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
                consumed_cost: InstructionCost::new(budget.raw()),
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
fn commit_validation_failure_traces_as_fault_discarded() {
    use cellgov_effects::WritePayload;
    use cellgov_event::PriorityClass;
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_trace::{TraceReader, TraceRecord};

    #[derive(Clone)]

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
                range: ByteRange::new(GuestAddr::new(1024), 4).unwrap(),
                bytes: WritePayload::new(vec![0; 4]),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(budget.raw()),
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
    let _ = rt.commit_step(&s.result, &s.effects).unwrap_err();
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
            // Invariant: epoch advances on every commit boundary, including faults.
            assert_eq!(*epoch_after, Epoch::new(1));
        }
        _ => unreachable!(),
    }
}

#[test]
fn commit_reserved_write_traces_as_fault_discarded() {
    // Staging-path counterpart to commit_validation_failure_traces_as_fault_discarded:
    // covers the ReservedWrite branch of the shared validate_write
    // predicate, where the unmapped test only covers the Unmapped
    // branch. Together they ensure both branches of the shared
    // predicate gate the staging-path commit.
    use cellgov_effects::WritePayload;
    use cellgov_event::PriorityClass;
    use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};
    use cellgov_trace::{TraceReader, TraceRecord};

    #[derive(Clone)]
    struct ReservedTargetUnit {
        id: UnitId,
        done: Cell<bool>,
    }
    impl ExecutionUnit for ReservedTargetUnit {
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
                range: ByteRange::new(GuestAddr::new(0x10000), 4).unwrap(),
                bytes: WritePayload::new(vec![0xAB; 4]),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }
        fn snapshot(&self) {}
    }

    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x10000, "rw", PageSize::Page64K),
        Region::with_access(
            0x10000,
            0x10000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    rt.registry_mut().register_with(|id| ReservedTargetUnit {
        id,
        done: Cell::new(false),
    });
    let s = rt.step().unwrap();
    let _ = rt.commit_step(&s.result, &s.effects).unwrap_err();
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
            writes_committed,
            fault_discarded,
            ..
        } => {
            assert_eq!(*writes_committed, 0);
            assert!(*fault_discarded);
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
    assert!(
        records.len() >= 7,
        "FullTrace mode should emit >= 7 trace records, got {}",
        records.len()
    );
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
