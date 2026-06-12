//! DMA enqueue/completion against reservations and SPU tag-wait wake ordering.

use super::*;

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
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(100)), None);
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.dma_completions_fired, 0);
    assert_eq!(rt.dma_queue().len(), 1);
}

#[test]
fn dma_enqueue_rejects_reserved_destination_preserves_reservation() {
    // pre_validate rejects a DmaEnqueue whose destination lies in a
    // non-ReadWrite region. The atomic-batch contract discards the
    // batch, so no completion fires and the cross-unit reservation
    // survives because the DMA was rejected upstream.
    use crate::commit::CommitError;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};

    fn run() -> (bool, bool, Vec<u8>) {
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
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x80), 4).unwrap(), &[0x11; 4])
            .unwrap();
        rt.reservations_mut().insert_or_replace(
            UnitId::new(1),
            cellgov_sync::ReservedLine::containing(0x10000),
        );
        let ops = vec![
            FakeOp::DmaPut {
                src: 0x80,
                dst: 0x10000,
                len: 4,
            },
            FakeOp::End,
        ];
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, ops));
        let step = rt.step().unwrap();
        let err = rt.commit_step(&step.result, &step.effects).unwrap_err();
        let rejected = matches!(
            err,
            CommitError::DmaDestinationReserved {
                effect_index: _,
                addr: 0x10000,
                region: "reserved",
            }
        );
        let cross_unit_held = rt.reservations().is_held_by(UnitId::new(1));
        (rejected, cross_unit_held, rt.trace().bytes().to_vec())
    }

    let (rejected_a, held_a, trace_a) = run();
    let (rejected_b, held_b, trace_b) = run();

    assert!(
        rejected_a,
        "enqueue must reject the DmaEnqueue with DmaDestinationReserved"
    );
    assert!(
        held_a,
        "cross-unit reservation must survive the rejected batch"
    );
    assert_eq!(
        rejected_a, rejected_b,
        "rejection observable must be stable"
    );
    assert_eq!(held_a, held_b, "reservation observable must be stable");
    assert_eq!(trace_a, trace_b, "trace bytes must be byte-identical");
}

#[test]
fn dma_completion_clears_overlapping_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    {
        use cellgov_mem::{ByteRange, GuestAddr};
        let range = ByteRange::new(GuestAddr::new(0x80), 4).unwrap();
        rt.memory_mut().apply_commit(range, &[0x11; 4]).unwrap();
    }
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

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

#[test]
fn dma_wait_parks_spu_blocked_and_completion_wakes_it() {
    use cellgov_exec::UnitStatus;
    use cellgov_mem::{ByteRange, GuestAddr};

    fn run() -> (UnitStatus, (u32, UnitStatus), Vec<u8>) {
        let mut rt = build(0x1000, 1, 200);
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x80), 4).unwrap(), &[0x11; 4])
            .unwrap();
        let _unit_id = rt.registry_mut().register_with(|id| TagPollUnit {
            id,
            step: Cell::new(0),
            seen_tag_bits: Cell::new(0),
            dst_addr: 0x100,
        });

        let mut observed_blocked_during_wait = false;
        let mut completions_fired = 0usize;
        let mut final_status = UnitStatus::Runnable;
        for _ in 0..200 {
            match rt.step() {
                Ok(step) => {
                    let outcome = rt.commit_step(&step.result, &step.effects).unwrap();
                    completions_fired += outcome.dma_completions_fired;
                    if step.result.yield_reason == YieldReason::DmaWait {
                        if let Some(status) = rt.registry().effective_status(step.unit) {
                            if status == UnitStatus::Blocked {
                                observed_blocked_during_wait = true;
                            }
                        }
                    }
                    final_status = rt
                        .registry()
                        .effective_status(step.unit)
                        .unwrap_or(UnitStatus::Runnable);
                    if final_status == UnitStatus::Finished {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        (
            if observed_blocked_during_wait {
                UnitStatus::Blocked
            } else {
                UnitStatus::Runnable
            },
            (completions_fired as u32, final_status),
            rt.trace().bytes().to_vec(),
        )
    }

    let (blocked_a, (_fired_a, final_a), trace_a) = run();
    let (blocked_b, (_fired_b, final_b), trace_b) = run();

    assert_eq!(
        blocked_a,
        UnitStatus::Blocked,
        "SPU must be Blocked while DmaWait yields and the tag bit is unset"
    );
    assert_eq!(
        final_a,
        UnitStatus::Finished,
        "the wake must let the SPU resume past the tag poll and reach Finished -- \
         if this fails with the SPU still Blocked, the completion path published \
         the tag bit but did NOT transition Blocked -> Runnable (the wake is the \
         load-bearing half of the change)"
    );
    assert_eq!(blocked_a, blocked_b, "Blocked observation deterministic");
    assert_eq!(final_a, final_b, "final status deterministic");
    assert_eq!(trace_a, trace_b, "trace deterministic across two runs");
}

/// Same-commit ordering pin. Per-step budget exceeds the DMA's modeled
/// latency, so the SPU's DmaWait yield and its outstanding completion
/// land in the SAME commit_step: park happens, then fire publishes the
/// tag bit AND overrides Blocked back to Runnable -- one commit, two
/// transitions. Mutation: swapping commit_step's park-before-fire
/// ordering to fire-before-park makes this test hang at AllBlocked
/// because fire's Runnable override gets overwritten by the late
/// Blocked, and the queue is now empty so time-warp can't recover.
#[test]
fn dma_wait_same_commit_completion_overrides_blocked_to_runnable() {
    use cellgov_exec::UnitStatus;
    use cellgov_mem::{ByteRange, GuestAddr};

    fn run() -> (UnitStatus, Vec<u8>) {
        // Budget 20 > FixedLatency(10): step-1's DmaWait yield consumes
        // 20 ticks, so the PUT issued in step 0 (completion_time = 1 + 10)
        // is already due at the moment step 1's commit_step runs
        // fire_dma_completions.
        let mut rt = build(0x1000, 20, 200);
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x80), 4).unwrap(), &[0x11; 4])
            .unwrap();
        rt.registry_mut().register_with(|id| TagPollUnit {
            id,
            step: Cell::new(0),
            seen_tag_bits: Cell::new(0),
            dst_addr: 0x100,
        });

        let mut final_status = UnitStatus::Runnable;
        for _ in 0..50 {
            match rt.step() {
                Ok(step) => {
                    rt.commit_step(&step.result, &step.effects).unwrap();
                    final_status = rt
                        .registry()
                        .effective_status(step.unit)
                        .unwrap_or(UnitStatus::Runnable);
                    if final_status == UnitStatus::Finished {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        (final_status, rt.trace().bytes().to_vec())
    }

    let (final_a, trace_a) = run();
    let (final_b, trace_b) = run();

    assert_eq!(
        final_a,
        UnitStatus::Finished,
        "park-before-fire ordering must let the same-commit completion override \
         Blocked back to Runnable -- if this fails Blocked, the ordering was \
         reversed and fire's wake got overwritten"
    );
    assert_eq!(final_a, final_b, "same-commit ordering deterministic");
    assert_eq!(trace_a, trace_b, "trace bytes byte-identical across runs");
}

/// Pre-validate rejection of an SPU-shape DMA must terminate the issuer,
/// not leave it polling a tag bit that will never publish. Without the
/// status mutation at the rejection site, the SPU's next step yields
/// DmaWait, parks Blocked, then `step` time-warps to an empty dma_queue
/// and returns `StepError::AllBlocked` -- a forever-stall. With the
/// mutation, the SPU is `Faulted` and the scheduler returns
/// `NoRunnableUnit` at the next step. The `CommitError` is unchanged
/// (the host-visible signal is preserved).
#[test]
fn dma_enqueue_rejection_faults_issuer_instead_of_stalling() {
    use crate::commit::CommitError;
    use crate::runtime::StepError;
    use cellgov_exec::UnitStatus;
    use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};

    fn run() -> (CommitErrShape, Option<UnitStatus>, StepError, Vec<u8>) {
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
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x80), 4).unwrap(), &[0x11; 4])
            .unwrap();
        let unit_id = rt.registry_mut().register_with(|id| TagPollUnit {
            id,
            step: Cell::new(0),
            seen_tag_bits: Cell::new(0),
            dst_addr: 0x10000,
        });

        // Step 1: SPU yields DmaSubmitted with the DmaEnqueue to the
        // reserved region; commit_step rejects.
        let step_a = rt.step().expect("first step runs");
        let err = rt
            .commit_step(&step_a.result, &step_a.effects)
            .expect_err("rejected batch");
        let err_shape = match err {
            CommitError::DmaDestinationReserved { addr, region, .. } => {
                CommitErrShape::Reserved { addr, region }
            }
            other => CommitErrShape::Other(format!("{other:?}")),
        };

        let status_after_reject = rt.registry().effective_status(unit_id);

        // Step 2: with the mutation, the SPU is Faulted; scheduler skips
        // it, no Blocked units, so NoRunnableUnit. Without the mutation,
        // the SPU is still Runnable, polls the tag bit (never published),
        // yields DmaWait, parks Blocked; the time-warp finds an empty
        // queue and returns AllBlocked.
        let terminal = rt.step().expect_err("no further progress possible");

        (
            err_shape,
            status_after_reject,
            terminal,
            rt.trace().bytes().to_vec(),
        )
    }

    let (err_a, status_a, terminal_a, trace_a) = run();
    let (err_b, status_b, terminal_b, trace_b) = run();

    assert!(
        matches!(
            err_a,
            CommitErrShape::Reserved {
                addr: 0x10000,
                region: "reserved"
            }
        ),
        "commit_step must still return the typed DmaDestinationReserved \
         (the host-visible signal is preserved): got {err_a:?}"
    );
    assert_eq!(
        status_a,
        Some(UnitStatus::Faulted),
        "the SPU issuer must be Faulted after pre-validate rejection -- \
         without this status mutation, the SPU rolls into a tag-poll \
         DmaWait that can never wake, and the runtime hangs at AllBlocked"
    );
    assert_eq!(
        terminal_a,
        StepError::NoRunnableUnit,
        "with the issuer Faulted, the runtime terminates at NoRunnableUnit \
         on the next step -- NOT AllBlocked. AllBlocked would mean the \
         status mutation did not fire and the SPU stalled."
    );
    assert_eq!(err_a, err_b, "rejection shape deterministic");
    assert_eq!(status_a, status_b, "issuer status deterministic");
    assert_eq!(terminal_a, terminal_b, "terminal deterministic");
    assert_eq!(trace_a, trace_b, "trace bytes byte-identical across runs");
}

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

#[test]
fn conditional_store_through_runtime_clears_own_and_overlapping() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    let mut rt = build(256, 1, 100);
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(0), cellgov_sync::ReservedLine::containing(0));
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), cellgov_sync::ReservedLine::containing(0));

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
