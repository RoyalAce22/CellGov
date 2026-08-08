//! FaultDriven trivial-step fast path: epoch advance, slow-path deferral,
//! wake visibility, and scheduler-notification parity with the slow path.

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

/// Two silent units, unit 0 holding an lwmutex it never releases.
///
/// Every step is trivial, so under `FaultDriven` the whole run goes
/// through the fast path and only the arguments it passes to
/// `notify_yielded` decide the rotation. Any other mode takes the slow
/// path, which makes `mode` the A/B lever.
fn rotation_holding_lwmutex(mode: RuntimeMode, steps: usize) -> Vec<UnitId> {
    use cellgov_lv2::ppu_thread::PpuThreadAttrs;

    let mut rt = build(64, 1, steps + 16);
    rt.set_mode(mode);
    for _ in 0..2 {
        rt.registry_mut().register_with(|id| SilentUnit {
            id,
            steps: Cell::new(0),
            max: u64::MAX,
        });
    }

    let holder = UnitId::new(0);
    rt.lv2_host.seed_primary_ppu_thread(
        holder,
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        },
    );
    let tid = rt
        .lv2_host
        .ppu_thread_id_for_unit(holder)
        .expect("seeded primary thread maps to unit 0");
    rt.lv2_host.lwmutex_holds_inc(tid);

    let mut selected = Vec::with_capacity(steps);
    for _ in 0..steps {
        let s = rt.step().expect("two runnable units, budget unexhausted");
        selected.push(s.unit);
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    selected
}

// F-01: the fast path passes `holds_critical_section` from the lwmutex
// table rather than a hardcoded false. Hardcoding it rotates 0,1,0,1
// where the scheduler should stick.
#[test]
fn fast_path_reports_held_lwmutex_to_the_scheduler() {
    let selected = rotation_holding_lwmutex(RuntimeMode::FaultDriven, 32);
    assert!(
        selected.iter().all(|&u| u == UnitId::new(0)),
        "unit 0 holds an lwmutex, so the scheduler must stay sticky \
         across trivial steps; got {selected:?}"
    );
}

// F-01: the streak counter advances on the fast path too. The run
// crosses STICKY_STREAK_LIMIT, so a fast path that skipped the
// increment would hold unit 0 past the point where the slow path
// releases it.
#[test]
fn fast_path_rotation_matches_slow_path_across_the_sticky_limit() {
    let fast = rotation_holding_lwmutex(RuntimeMode::FaultDriven, 200);
    let slow = rotation_holding_lwmutex(RuntimeMode::FullTrace, 200);
    assert_eq!(
        fast, slow,
        "fast-path rotation diverged from slow-path rotation"
    );
    assert!(
        slow.contains(&UnitId::new(1)),
        "200 steps must cross the sticky-streak limit and release unit 0, \
         otherwise this pins nothing about the streak counter"
    );
}
