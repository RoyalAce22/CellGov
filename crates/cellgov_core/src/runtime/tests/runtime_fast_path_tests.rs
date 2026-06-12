//! FaultDriven trivial-step fast path: epoch advance, slow-path deferral, wake visibility.

use super::*;

#[test]
fn commit_fast_path_empty_loop_advances_epoch_monotonically() {
    let mut rt = build(64, 1, 20_000);
    rt.set_mode(RuntimeMode::FaultDriven);
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
    let epoch_before = rt.epoch();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.epoch().raw(), epoch_before.raw() + 1);
}

// Invariant: status_overrides survives the fast path, so a DMA wake on a
// blocked unit stays observable through another unit's silent steps.
#[test]
fn commit_fast_path_preserves_wake_visibility_through_silent_steps() {
    use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};

    let mut rt = build(256, 1, 100);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(|id| SilentUnit {
        id,
        steps: Cell::new(0),
        max: 100,
    });
    rt.registry_mut()
        .set_status_override(UnitId::new(0), UnitStatus::Blocked);
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
