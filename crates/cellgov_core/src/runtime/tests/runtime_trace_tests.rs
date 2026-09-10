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
        full_per_step: vec![vec![(
            1,
            0x104,
            cellgov_exec::PpuFingerprint {
                gpr: [0u64; 32],
                lr: 0,
                ctr: 0,
                xer: 0,
                cr: 0,
                reservation_line: None,
            },
        )]],
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
        TraceRecord::PpuStateFull { step, pc, .. } => {
            assert_eq!(*pc, 0x104);
            assert_eq!(
                *step, 1,
                "snapshot must carry the unit's retirement counter, \
                 aligning with the hash record for the same instruction"
            );
        }
        other => panic!("expected PpuStateFull, got {other:?}"),
    }
}

/// Reads the reserved-zero region at `0xC000_0010` twice and
/// `0xC000_0020` once per step through the frozen context. With
/// `faults` set, every step yields `Fault` after those reads.
#[derive(Clone)]
struct ReservedReadingUnit {
    id: UnitId,
    steps: Cell<u64>,
    max: u64,
    faults: bool,
}

impl ExecutionUnit for ReservedReadingUnit {
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
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        use cellgov_mem::{ByteRange, GuestAddr};
        let word = ByteRange::new(GuestAddr::new(0xC000_0010), 4).unwrap();
        let dword = ByteRange::new(GuestAddr::new(0xC000_0020), 8).unwrap();
        assert_eq!(ctx.memory().read(word), Some(&[0u8; 4][..]));
        assert_eq!(ctx.memory().read(word), Some(&[0u8; 4][..]));
        assert_eq!(ctx.memory().read(dword), Some(&[0u8; 8][..]));
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if self.faults {
            return ExecutionStepResult {
                yield_reason: YieldReason::Fault,
                consumed_cost: InstructionCost::ZERO,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: Some(cellgov_effects::FaultKind::Guest(0x700)),
                syscall_args: None,
            };
        }
        ExecutionStepResult {
            yield_reason: if n >= self.max {
                YieldReason::Finished
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn reserved_runtime(mode: RuntimeMode) -> Runtime {
    reserved_runtime_with(mode, false)
}

fn reserved_runtime_with(mode: RuntimeMode, faults: bool) -> Runtime {
    use cellgov_mem::{PageSize, Region, RegionAccess};
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "flat", PageSize::Page64K),
        Region::with_access(
            0xC000_0000,
            0x1000,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut rt = Runtime::new(mem, Budget::new(4), 100);
    rt.set_mode(mode);
    rt.registry_mut().register_with(|id| ReservedReadingUnit {
        id,
        steps: Cell::new(0),
        max: 2,
        faults,
    });
    rt
}

fn reserved_reads(rt: &Runtime) -> Vec<(u64, u64, u64, u32, u32)> {
    use cellgov_trace::{TraceReader, TraceRecord};
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::ReservedRegionRead {
                unit,
                step,
                addr,
                len,
                hits,
            } => Some((unit.raw(), step, addr, len, hits)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reserved_read_during_a_step_is_traced_by_unit_step_address_and_hits() {
    let mut rt = reserved_runtime(RuntimeMode::FullTrace);
    rt.step().unwrap();
    assert_eq!(
        reserved_reads(&rt),
        vec![(0, 1, 0xC000_0010, 4, 2), (0, 1, 0xC000_0020, 8, 1)]
    );
    assert!(
        rt.memory().drain_provisional_reads().is_empty(),
        "the step drained the log; nothing leaks into the next step"
    );
    rt.step().unwrap();
    assert_eq!(
        reserved_reads(&rt).len(),
        4,
        "the second step traces its own reads under step 2"
    );
    assert_eq!(reserved_reads(&rt)[2], (0, 2, 0xC000_0010, 4, 2));
}

#[test]
fn determinism_check_mode_traces_reserved_reads_beside_the_hash_stream() {
    let mut rt = reserved_runtime(RuntimeMode::DeterminismCheck);
    rt.step().unwrap();
    assert_eq!(reserved_reads(&rt).len(), 2);
}

#[test]
fn fault_driven_mode_drains_reserved_reads_without_tracing_them() {
    let mut rt = reserved_runtime(RuntimeMode::FaultDriven);
    rt.step().unwrap();
    assert!(reserved_reads(&rt).is_empty());
    assert!(
        rt.memory().drain_provisional_reads().is_empty(),
        "the log is drained even when nothing is traced"
    );
    assert_eq!(
        rt.memory().provisional_read_count(),
        3,
        "the running count still reports the reads"
    );
}

#[test]
fn a_host_side_read_between_step_and_commit_is_traced_at_the_commit_under_the_committing_unit() {
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_trace::{TraceReader, TraceRecord};
    let mut rt = reserved_runtime(RuntimeMode::FullTrace);
    let s1 = rt.step().unwrap();
    let step_reads = reserved_reads(&rt).len();
    // An LV2 arm or the RSX model reading on the committing unit's
    // behalf goes through the same `GuestMemory::read` path.
    let host = ByteRange::new(GuestAddr::new(0xC000_0040), 16).unwrap();
    assert_eq!(rt.memory().read(host), Some(&[0u8; 16][..]));
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let reads = reserved_reads(&rt);
    assert_eq!(reads.len(), step_reads + 1);
    assert_eq!(
        reads[step_reads],
        (0, 1, 0xC000_0040, 16, 1),
        "attributed to the committing unit at the step count the commit closes"
    );
    let records: Vec<TraceRecord> = TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .collect();
    let commit_pos = records
        .iter()
        .position(|r| matches!(r, TraceRecord::CommitApplied { .. }))
        .expect("CommitApplied present");
    let host_read_pos = records
        .iter()
        .position(|r| {
            matches!(
                r,
                TraceRecord::ReservedRegionRead {
                    addr: 0xC000_0040,
                    ..
                }
            )
        })
        .expect("host-side read traced");
    assert!(
        host_read_pos < commit_pos,
        "the read lands inside the commit's window, before CommitApplied"
    );
}

#[test]
fn a_faulting_step_still_traces_the_reserved_reads_it_consumed_before_the_fault() {
    use cellgov_trace::{TraceReader, TraceRecord};
    let mut rt = reserved_runtime_with(RuntimeMode::FullTrace, true);
    let s1 = rt.step().unwrap();
    assert_eq!(
        reserved_reads(&rt),
        vec![(0, 1, 0xC000_0010, 4, 2), (0, 1, 0xC000_0020, 8, 1)],
        "fault-discards-all covers the batch's effects; the zeros the unit \
         consumed before faulting are part of the run and stay located"
    );
    rt.commit_step(&s1.result, &s1.effects).unwrap();
    let fault_discarded = TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .find_map(|r| match r {
            TraceRecord::CommitApplied {
                fault_discarded, ..
            } => Some(fault_discarded),
            _ => None,
        })
        .expect("CommitApplied present");
    assert!(fault_discarded);
    assert_eq!(
        reserved_reads(&rt).len(),
        2,
        "the commit adds no reads of its own"
    );
}

fn syscall_records(rt: &Runtime) -> Vec<(bool, u64, u64, u64)> {
    use cellgov_trace::{TraceReader, TraceRecord};
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::SyscallEntered { unit, num, .. } => Some((true, unit.raw(), num, 0)),
            TraceRecord::SyscallReturned { unit, code, time } => {
                Some((false, unit.raw(), code, time.raw()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_immediate_syscall_return_is_traced_in_the_dispatching_commit() {
    let mut rt = build(4096, 4, 100);
    let mut args = [0u64; 9];
    args[0] = 999;
    let unit_id = rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args: args,
    });
    let s = rt.step().unwrap();
    assert!(
        syscall_records(&rt).iter().all(|r| r.0),
        "nothing is returned before the commit dispatches the call"
    );
    rt.commit_step(&s.result, &s.effects).unwrap();

    let records = syscall_records(&rt);
    assert_eq!(records.len(), 2, "one entry and one return: {records:?}");
    assert_eq!(records[0], (true, unit_id.raw(), 999, 0));
    let (is_entry, unit, code, time) = records[1];
    assert!(!is_entry);
    assert_eq!(unit, unit_id.raw());
    assert_eq!(
        time,
        rt.time().raw(),
        "delivered at the dispatching commit's clock"
    );
    let delivered = rt
        .registry_mut()
        .drain_syscall_return(unit_id)
        .expect("the value is pending for the caller's next step");
    assert_eq!(
        code, delivered,
        "the traced value is the one the caller will see in r3"
    );
}

/// Parks on `sys_timer_usleep(50)` and finishes on its next scheduling.
#[derive(Clone)]
struct ParkingUnit {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for ParkingUnit {
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
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            let mut args = [0u64; 9];
            args[0] = cellgov_ps3_abi::lv2::syscall::TIMER_USLEEP;
            args[1] = 50;
            ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::with_pc(0x1000),
                fault: None,
                syscall_args: Some(args),
            }
        } else {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(1),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn a_blocked_syscall_traces_its_return_at_wake_time_not_at_dispatch() {
    let mut rt = build(4096, 16, 100);
    let unit_id = rt.registry_mut().register_with(|id| ParkingUnit {
        id,
        steps: Cell::new(0),
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let park_time = rt.time();
    assert!(
        syscall_records(&rt).iter().all(|r| r.0),
        "a parked call has no return to trace yet"
    );

    // Everything is parked: this step time-warps to the deadline,
    // fires the wake, and delivers r3 before the sleeper runs.
    let s2 = rt.step().unwrap();
    assert_eq!(s2.unit, unit_id);
    let returns: Vec<_> = syscall_records(&rt).into_iter().filter(|r| !r.0).collect();
    assert_eq!(returns.len(), 1, "{returns:?}");
    let (_, unit, code, time) = returns[0];
    assert_eq!(unit, unit_id.raw());
    assert_eq!(code, 0, "usleep completes with CELL_OK");
    assert_eq!(
        time,
        park_time.raw() + 50_000,
        "delivered at the deadline the time-warp jumped to"
    );
}

#[test]
fn fault_driven_mode_delivers_syscall_returns_without_tracing_them() {
    let mut rt = build(4096, 4, 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    let mut args = [0u64; 9];
    args[0] = 999;
    let unit_id = rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args: args,
    });
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert!(syscall_records(&rt).is_empty());
    assert!(
        rt.registry_mut().drain_syscall_return(unit_id).is_some(),
        "the value still reaches the caller"
    );
}
