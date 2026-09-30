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
fn an_untagged_completion_wakes_its_issuer_with_no_tag_bit() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let issuer = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .set_status_override(issuer, cellgov_exec::UnitStatus::Blocked);

    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        issuer,
    )
    .unwrap();
    assert_eq!(req.tag_id(), None, "the premise is an untagged request");
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    assert_eq!(
        rt.outstanding_dma_tags(issuer),
        0,
        "a queued transfer with no tag holds no tag group outstanding",
    );

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.registry().effective_status(issuer),
        Some(cellgov_exec::UnitStatus::Runnable),
        "the completion un-parks the issuer whether or not it carried a tag",
    );
}

/// Uses tag 5 because at tag 0 the shift `1 << 0` equals the constant 1.
///
/// [CBEA p:126 s:9.3.4 MFC Read Tag-Group Query Mask Channel] the mask's bit positions run g1F..g0, so tag group n is the bit of weight 2^n.
#[test]
fn a_queued_tagged_transfer_holds_that_tag_groups_bit_until_it_completes() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ps3_abi::hw::spu::MfcTagId;
    let mut rt = build(256, 5, 100);
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let issuer = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .set_status_override(issuer, cellgov_exec::UnitStatus::Blocked);

    let tag = MfcTagId::new(5).expect("5 is inside the architected range");
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        issuer,
    )
    .unwrap()
    .with_tag_id(tag);
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(3)), None);
    assert_eq!(
        rt.outstanding_dma_tags(issuer),
        0x20,
        "tag group 5 is the bit of weight 32",
    );

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.outstanding_dma_tags(issuer),
        0,
        "the completed transfer leaves its group with nothing outstanding",
    );
}

/// [CBEA p:128 s:9.3.6] a tag group reads complete when it has no outstanding operations.
#[test]
fn outstanding_tag_groups_count_only_the_units_own_transfers_and_a_reused_tag_stays_outstanding() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ps3_abi::hw::spu::MfcTagId;
    let mut rt = build(256, 5, 100);
    let a = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let b = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    let tagged = |issuer, tag| {
        DmaRequest::new(
            DmaDirection::Put,
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            ByteRange::new(GuestAddr::new(128), 4).unwrap(),
            issuer,
        )
        .unwrap()
        .with_tag_id(MfcTagId::new(tag).expect("tag in range"))
    };
    rt.dma_queue
        .enqueue(DmaCompletion::new(tagged(a, 2), GuestTicks::new(3)), None);
    rt.dma_queue
        .enqueue(DmaCompletion::new(tagged(a, 2), GuestTicks::new(50)), None);
    rt.dma_queue
        .enqueue(DmaCompletion::new(tagged(b, 4), GuestTicks::new(3)), None);
    assert_eq!(
        rt.outstanding_dma_tags(a),
        1 << 2,
        "unit b's tag 4 is not a's"
    );
    assert_eq!(
        rt.outstanding_dma_tags(b),
        1 << 4,
        "unit a's tag 2 is not b's"
    );

    rt.time = GuestTicks::new(3);
    let fired = rt.fire_dma_completions();
    assert_eq!(
        fired.len(),
        2,
        "the premise is that both time-3 transfers land"
    );
    assert_eq!(
        rt.outstanding_dma_tags(a),
        1 << 2,
        "a's second tag-2 transfer is still queued, so the group stays outstanding",
    );
    assert_eq!(rt.outstanding_dma_tags(b), 0);
}

#[test]
fn a_dma_completion_for_a_finished_issuer_does_not_resurrect_it() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 5, 100);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0xaa, 0xbb, 0xcc, 0xdd],
        )
        .unwrap();
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    // The issuer finished (process-exit sweep shape) with its DMA
    // still in flight.
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Finished);
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
    // The in-flight payload still lands in the terminal memory image.
    assert_eq!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
            .unwrap(),
        &[0xaa, 0xbb, 0xcc, 0xdd]
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(cellgov_exec::UnitStatus::Finished),
        "a finished issuer must not be overridden back to Runnable by a late completion"
    );
    // No status transition happened, so no UnitWoken record either:
    // tracing one would fabricate a wake the guard suppressed.
    let bytes = rt.trace().bytes().to_vec();
    let woke_finished = cellgov_trace::TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .any(|r| {
            matches!(
                r,
                cellgov_trace::TraceRecord::UnitWoken { unit, .. } if unit == UnitId::new(1)
            )
        });
    assert!(
        !woke_finished,
        "a suppressed wake for a Finished issuer must not emit UnitWoken"
    );
}

#[test]
fn a_dma_completion_for_a_faulted_issuer_does_not_resurrect_it() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 5, 100);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0x55, 0x66, 0x77, 0x88],
        )
        .unwrap();
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    // The issuer faulted (pre-validate rejection shape) after an
    // earlier accepted transfer was already in flight.
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Faulted);
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
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
            .unwrap(),
        &[0x55, 0x66, 0x77, 0x88],
        "the accepted transfer still lands"
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(cellgov_exec::UnitStatus::Faulted),
        "the Faulted mark keeps the issuer off the scheduler; a late completion \
         must not override it back to Runnable"
    );
}

#[test]
fn a_time_warp_over_a_finished_issuers_completion_terminates_at_all_blocked() {
    use crate::runtime::StepError;
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    let mut rt = build(256, 5, 100);
    rt.memory
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0x11, 0x22, 0x33, 0x44],
        )
        .unwrap();
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 10));
    rt.registry_mut()
        .set_status_override(UnitId::new(0), cellgov_exec::UnitStatus::Blocked);
    rt.registry_mut()
        .set_status_override(UnitId::new(1), cellgov_exec::UnitStatus::Finished);
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0), 4).unwrap(),
        ByteRange::new(GuestAddr::new(128), 4).unwrap(),
        UnitId::new(1),
    )
    .unwrap();
    rt.dma_queue
        .enqueue(DmaCompletion::new(req, GuestTicks::new(50)), None);
    let err = rt.step().expect_err("no unit can become runnable");
    assert_eq!(err, StepError::AllBlocked);
    assert_eq!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
            .unwrap(),
        &[0x11, 0x22, 0x33, 0x44],
        "the in-flight payload still lands during the warp"
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(cellgov_exec::UnitStatus::Finished)
    );
    assert!(
        rt.time().raw() >= 50,
        "the warp advanced to the completion time"
    );
    let bytes = rt.trace().bytes().to_vec();
    let any_woken = cellgov_trace::TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .any(|r| matches!(r, cellgov_trace::TraceRecord::UnitWoken { .. }));
    assert!(
        !any_woken,
        "warp over a suppressed wake must not emit UnitWoken for anyone"
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

/// The raised put writes nothing, so another unit's reservation over the
/// line survives.
#[test]
fn a_put_to_a_reserved_destination_raises_and_preserves_reservation() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};

    fn run() -> (Option<cellgov_dma::MfcCommandError>, bool, Vec<u8>) {
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
        for _ in 0..8 {
            let Ok(step) = rt.step() else { break };
            rt.commit_step(&step.result, &step.effects)
                .expect("the enqueue is accepted");
        }
        rt.drain_pending_dma();
        let raised = rt.take_mfc_exception().map(|e| e.command.error);
        let cross_unit_held = rt.reservations().is_held_by(UnitId::new(1));
        (raised, cross_unit_held, rt.trace().bytes().to_vec())
    }

    let (raised_a, held_a, trace_a) = run();
    let (raised_b, held_b, trace_b) = run();

    assert_eq!(
        raised_a,
        Some(cellgov_dma::MfcCommandError::DataStorage { ea: 0x10000 }),
        "the queue raises the put when it reaches it"
    );
    assert!(
        held_a,
        "cross-unit reservation must survive a put that wrote nothing"
    );
    assert_eq!(raised_a, raised_b, "the raise is stable");
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
        "SPU must be Blocked while DmaWait yields and its tag group is outstanding"
    );
    assert_eq!(
        final_a,
        UnitStatus::Finished,
        "the wake must let the SPU resume past the tag poll and reach Finished -- \
         if this fails with the SPU still Blocked, the completion path landed \
         the transfer but did not move Blocked to Runnable"
    );
    assert_eq!(blocked_a, blocked_b, "Blocked observation deterministic");
    assert_eq!(final_a, final_b, "final status deterministic");
    assert_eq!(trace_a, trace_b, "trace deterministic across two runs");
}

#[test]
fn dma_wait_same_commit_completion_overrides_blocked_to_runnable() {
    use cellgov_exec::UnitStatus;
    use cellgov_mem::{ByteRange, GuestAddr};

    fn run() -> (UnitStatus, bool, Vec<u8>) {
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
        let mut parked = false;
        for _ in 0..50 {
            match rt.step() {
                Ok(step) => {
                    parked |= step.result.yield_reason == YieldReason::DmaWait;
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
        (final_status, parked, rt.trace().bytes().to_vec())
    }

    let (final_a, parked_a, trace_a) = run();
    let (final_b, _, trace_b) = run();

    // A transfer that never reaches the queue reads complete at once,
    // so the unit finishes with no DmaWait park.
    assert!(
        parked_a,
        "the premise is a DmaWait park on the queued transfer"
    );
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

/// A put the queue raises keeps its tag outstanding, so nothing wakes the
/// issuer that waits on the tag.
#[test]
fn a_raised_put_holds_its_tag_and_its_waiting_issuer_stays_parked() {
    use crate::runtime::StepError;
    use cellgov_exec::UnitStatus;
    use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};

    type Outcome = (
        Option<cellgov_dma::MfcCommandError>,
        Option<UnitStatus>,
        StepError,
        Vec<u8>,
    );

    fn run() -> Outcome {
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
        let terminal = loop {
            match rt.step() {
                Ok(step) => {
                    rt.commit_step(&step.result, &step.effects)
                        .expect("the enqueue is accepted");
                }
                Err(err) => break err,
            }
        };
        (
            rt.take_mfc_exception().map(|e| e.command.error),
            rt.registry().effective_status(unit_id),
            terminal,
            rt.trace().bytes().to_vec(),
        )
    }

    let (raised_a, status_a, terminal_a, trace_a) = run();
    let (raised_b, status_b, terminal_b, trace_b) = run();

    assert_eq!(
        raised_a,
        Some(cellgov_dma::MfcCommandError::DataStorage { ea: 0x10000 })
    );
    assert_eq!(
        status_a,
        Some(UnitStatus::Blocked),
        "the tag never reads complete, so the issuer never wakes"
    );
    assert_eq!(terminal_a, StepError::AllBlocked);
    assert_eq!(raised_a, raised_b, "the raise is deterministic");
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
