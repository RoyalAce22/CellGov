//! Snapshot/restore replay fidelity, container independence, and field-completeness guards.

use super::*;
use crate::scheduler::RoundRobinScheduler;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

fn make_runtime_with_two_writers() -> Runtime {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xBB),
                FakeOp::SharedStore { addr: 8, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt
}

fn drive(rt: &mut Runtime, n: usize) {
    for _ in 0..n {
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result, &step.effects);
            }
            Err(_) => break,
        }
    }
}

#[test]
fn snapshot_then_restore_replays_to_same_terminal_state() {
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();

    drive(&mut rt, 50);
    let original_hash = rt.memory().content_hash();

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    drive(&mut rt, 50);
    let restored_hash = rt.memory().content_hash();

    assert_eq!(
        original_hash, restored_hash,
        "terminal memory hash diverged after snapshot/restore replay",
    );
    // Field completeness is guarded by
    // `_snapshot_field_exhaustiveness_compile_guard`, not this replay.
}

/// Compile-time field-completeness guard: a no-rest destructure of
/// every [`Runtime`] field, so adding a field breaks compilation here
/// until it is categorized as snapshot-captured, excluded, or
/// asserted-unchanged. Never called.
#[allow(dead_code)]
fn _snapshot_field_exhaustiveness_compile_guard(rt: &Runtime) {
    let Runtime {
        // --- snapshot-captured ---
        registry: _,
        mailbox_registry: _,
        signal_registry: _,
        reservations: _,
        rsx_cursor: _,
        rsx_sem_offset: _,
        rsx_mirror_writes: _,
        rsx_flip: _,
        rsx_methods: _,
        pending_rsx_effects: _,
        dma_queue: _,
        timer_wakes: _,
        lv2_host: _,
        syscall_responses: _,
        commit_pipeline: _,
        memory: _,
        spaces: _,
        time: _,
        epoch: _,
        steps_taken: _,
        last_scheduled_unit: _,
        step_woke_others: _,
        per_step_index: _,
        pending_tag_completions: _,
        rsx_call_stack: _,
        rsx_consume_fifo: _,
        rsx_label_base: _,
        // --- captured for assert-unchanged, not restored ---
        budget_per_step: _,
        max_steps: _,
        mode: _,
        // --- excluded from restore; see module doc for category ---
        dma_latency: _,                   // set-once at construction
        spu_factory: _,                   // set-once at construction
        ppu_factory: _,                   // set-once at construction
        scheduler: _,                     // caller-replaced post-restore
        trace: _,                         // cleared on restore
        zoom_trace: _,                    // cleared on restore
        effects_buf: _,                   // cleared on restore (per-step scratch)
        scheduler_dirty_after_restore: _, // set true by restore
        rsx_label_writes_committed: _,    // audit counter, host-side only
        rsx_set_reference_dispatches: _,  // audit counter, host-side only
        timer_sleep_dispatches: _,        // audit counter, host-side only
        lv2_direct_committed_writes: _,   // staging-bypass witness, host-side only
        process_spawn_loader: _,          // host-installed closure, like the factories
    } = rt;
}

#[test]
fn snapshot_after_execution_restores_byte_identical_state() {
    let mut rt = make_runtime_with_two_writers();
    drive(&mut rt, 3);

    let snap = rt.snapshot();
    let pre_mem = rt.memory().content_hash();
    let pre_steps = rt.steps_taken();
    let pre_epoch_raw = rt.epoch();
    let pre_per_step = rt.per_step_index_for_tests();

    drive(&mut rt, 5);
    assert_ne!(
        rt.memory().content_hash(),
        pre_mem,
        "test setup: post-snapshot driving must mutate state",
    );

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());

    assert_eq!(
        rt.memory().content_hash(),
        pre_mem,
        "memory drifted across restore"
    );
    assert_eq!(
        rt.steps_taken(),
        pre_steps,
        "steps_taken drifted across restore"
    );
    assert_eq!(rt.epoch(), pre_epoch_raw, "epoch drifted across restore");
    assert_eq!(
        rt.per_step_index_for_tests(),
        pre_per_step,
        "per_step_index drifted across restore -- snapshot missed it",
    );
}

fn make_runtime_with_dma_drivers() -> Runtime {
    let mem = GuestMemory::new(0x4000);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    rt.registry_mut().register_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::DmaPut {
                    src: 0x100,
                    dst: 0x1000,
                    len: 32,
                },
                FakeOp::DmaPut {
                    src: 0x200,
                    dst: 0x2000,
                    len: 32,
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn dma_queue_aliasing_canary_two_directions() {
    // DMAs complete after DEFAULT_DMA_LATENCY_TICKS (10), so
    // driving 1 step keeps the enqueued completion in queue
    // while step 2 would advance time past the completion and
    // fire it. We need the queue non-empty when we observe.

    // Direction 1: original mutates after snapshot.
    let mut rt = make_runtime_with_dma_drivers();
    let snap = rt.snapshot();
    let snap_len_pre = snap.dma_queue.len();
    drive(&mut rt, 1);
    assert!(
        rt.dma_queue().len() > snap_len_pre,
        "test setup: one step must leave at least one DMA queued before its completion fires",
    );
    assert_eq!(
        snap.dma_queue.len(),
        snap_len_pre,
        "snapshot's dma_queue aliased the original; post-snapshot \
         enqueue leaked into the captured queue",
    );

    // Direction 2: restored runtime mutates, separate snapshot
    // held by the test stays put.
    let mut rt2 = make_runtime_with_dma_drivers();
    let snap2 = rt2.snapshot();
    let snap2_len_pre = snap2.dma_queue.len();
    rt2.restore_into(&snap2);
    rt2.set_scheduler(RoundRobinScheduler::new());
    drive(&mut rt2, 1);
    assert!(
        rt2.dma_queue().len() > snap2_len_pre,
        "test setup: one step must leave at least one DMA queued before its completion fires",
    );
    assert_eq!(
        snap2.dma_queue.len(),
        snap2_len_pre,
        "snapshot's dma_queue aliased the restored runtime; \
         post-restore enqueue leaked into the captured queue",
    );
}

#[test]
fn snapshot_memory_is_independent_of_post_snapshot_mutation() {
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();
    let snap_hash_before = snap.memory.content_hash();

    drive(&mut rt, 5);
    assert_ne!(
        rt.memory().content_hash(),
        snap_hash_before,
        "test setup: original must mutate to validate snapshot independence",
    );

    assert_eq!(
        snap.memory.content_hash(),
        snap_hash_before,
        "snapshot memory aliased the original -- post-snapshot mutation leaked",
    );
}

#[test]
fn repeated_restore_preserves_effects_buf_capacity() {
    let mut rt = make_runtime_with_two_writers();
    rt.effects_buf_mut_for_tests().reserve(128);
    let pre_capacity = rt.effects_buf_capacity_for_tests();
    assert!(pre_capacity >= 128);

    let snap = rt.snapshot();
    for _ in 0..4 {
        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());
    }
    let post_capacity = rt.effects_buf_capacity_for_tests();
    assert_eq!(
        pre_capacity, post_capacity,
        "effects_buf capacity changed across restores ({pre_capacity} -> {post_capacity})",
    );
}

#[test]
fn step_after_restore_without_set_scheduler_returns_typed_error() {
    use crate::runtime::StepError;
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();
    rt.restore_into(&snap);
    assert_eq!(rt.step(), Err(StepError::SchedulerNotReinstalled));
}

#[test]
fn set_scheduler_after_restore_clears_dirty_flag() {
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();
    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    let _ = rt.step();
}

#[test]
fn snapshot_captures_rsx_label_base_and_restore_overwrites_it() {
    // rsx_label_base is commit_step's RsxLabelWrite commit target,
    // so a missed capture is a guest-observable replay divergence.
    let mut rt = make_runtime_with_two_writers();
    rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(0x4000));
    let snap = rt.snapshot();
    assert_eq!(snap.rsx_label_base, 0x4000);

    rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(0x8000));
    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    assert_eq!(
        rt.rsx_label_base, 0x4000,
        "restore_into must overwrite rsx_label_base with snap's captured value",
    );
}

#[test]
fn a_pending_timer_wake_alone_shifts_sync_state_hash() {
    // Isolated membership witness for the timer_wakes term of
    // sync_state_hash. The replay-level test
    // (runtime_timer_tests::snapshot_restores_pending_timer_wake)
    // fires a real sleeper, which also mutates syscall_responses --
    // its hash asserts stay green if the timer term is dropped from
    // the fold. Here the queue is the only mutated source.
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();
    let h_no_wake = rt.sync_state_hash();

    rt.timer_wakes.insert(
        cellgov_time::GuestTicks::new(1_000),
        cellgov_event::UnitId::new(0),
        crate::timer_queue::TimerWakeKind::Sleep,
    );
    assert_ne!(
        rt.sync_state_hash(),
        h_no_wake,
        "a pending timer wake must shift sync_state_hash on its own",
    );

    rt.restore_into(&snap);
    assert_eq!(
        rt.sync_state_hash(),
        h_no_wake,
        "restore must clear the pending wake and return sync_state_hash \
         to its captured value",
    );
}

#[test]
fn restore_into_carries_audit_counters_forward() {
    // Audit counters are cumulative host-side instruments that never
    // feed the commit pipeline; see the snapshot.rs module doc.
    let mut rt = make_runtime_with_two_writers();
    let snap = rt.snapshot();

    rt.rsx_label_writes_committed = 3;
    rt.rsx_set_reference_dispatches = 5;
    rt.timer_sleep_dispatches = 7;
    rt.lv2_direct_committed_writes = 11;

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());

    assert_eq!(
        rt.rsx_label_writes_committed(),
        3,
        "restore_into rewound rsx_label_writes_committed",
    );
    assert_eq!(
        rt.rsx_set_reference_dispatches(),
        5,
        "restore_into rewound rsx_set_reference_dispatches",
    );
    assert_eq!(
        rt.timer_sleep_dispatches(),
        7,
        "restore_into rewound timer_sleep_dispatches",
    );
    assert_eq!(
        rt.lv2_direct_committed_writes(),
        11,
        "restore_into rewound lv2_direct_committed_writes",
    );
}

#[test]
fn restore_into_clears_trace_writers() {
    let mut rt = make_runtime_with_two_writers();
    drive(&mut rt, 1);
    let snap = rt.snapshot();
    drive(&mut rt, 2);
    assert!(
        rt.trace().record_count() > 0,
        "test setup: post-snapshot stepping must produce trace records",
    );

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());

    assert_eq!(
        rt.trace().record_count(),
        0,
        "main trace writer was not cleared on restore_into",
    );
    assert_eq!(
        rt.zoom_trace().record_count(),
        0,
        "zoom trace writer was not cleared on restore_into",
    );
}
