//! Dispatch and wake paths that report a broken invariant to the host channel.
//!
//! The two displacement tests run in release builds only: a debug build
//! panics inside the table before the report runs.

use super::*;

#[cfg(not(debug_assertions))]
use cellgov_lv2::PendingResponse;

#[cfg(not(debug_assertions))]
fn idle_thread_attrs() -> cellgov_lv2::PpuThreadAttrs {
    cellgov_lv2::PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    }
}

/// A group left in `Created` is the finish-before-start state
/// `notify_spu_finished` rejects.
fn record_spu_in_unstarted_group(rt: &mut Runtime, unit: UnitId) {
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups
        .create(1)
        .expect("group id 1 is free in a fresh table");
    groups
        .record_spu(unit, gid, 0)
        .expect("a Created group accepts a slot record");
}

#[test]
fn the_process_exit_sweep_records_a_rejected_spu_notify() {
    let mut rt = build(0x1000, 1, 100);
    let unit = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    record_spu_in_unstarted_group(&mut rt, unit);

    rt.dispatch_lv2_request(cellgov_lv2::Lv2Request::ProcessExit { code: 0 }, unit);

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.process_exit_notify_spu_finished_failed"),
        1,
        "the sweep must report the rejected notify on the host channel; a count of 0 \
         means the divergence between the thread table and the group state left no \
         record a trace diff can find",
    );
}

#[test]
fn resolve_join_wakes_records_a_rejected_spu_notify() {
    let mut rt = build(0x1000, 1, 100);
    let spu = UnitId::new(99);
    record_spu_in_unstarted_group(&mut rt, spu);

    rt.resolve_join_wakes_for_test(spu);

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.resolve_join_wakes_notify_spu_finished_failed"),
        1,
        "the join path must report the rejected notify on the host channel; a count of 0 \
         means the finish silently woke no joiner",
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn a_block_over_a_live_park_records_the_displaced_response_and_deadline() {
    const MUTEX_ID: u32 = 4;
    let mut rt = build(0x1000, 1, 100);
    let holder = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let blocker = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    rt.lv2_host_mut()
        .seed_primary_ppu_thread(holder, idle_thread_attrs());
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(blocker, idle_thread_attrs())
        .expect("a second PPU thread fits in a fresh table");
    rt.lv2_host_mut()
        .mutexes_mut()
        .create_with_id(MUTEX_ID, cellgov_lv2::MutexAttrs::default())
        .expect("mutex id 4 is free in a fresh table");
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 0,
        },
        holder,
    );

    // A resolved wait clears both of these. Here the unit about to
    // block again still owns a pending response and a live deadline.
    let _ = rt
        .syscall_responses_mut()
        .insert(blocker, PendingResponse::ReturnCode { code: 0xABCD });
    let _ = rt.timer_wakes.insert(
        GuestTicks::new(1_000),
        blocker,
        crate::timer_queue::TimerWakeKind::Sleep,
    );

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 5,
        },
        blocker,
    );

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.handle_block_pending_response_displaced"),
        1,
        "the overwritten response owes an r3 and its out-pointer writes; a count of 0 \
         means both were dropped with no record",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.register_wait_deadline_timer_wake_displaced"),
        1,
        "the displaced deadline means an earlier wait never cancelled its wake; a count \
         of 0 means that uncancelled path stays invisible",
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn a_timer_park_over_a_live_park_records_the_displaced_response_and_deadline() {
    let mut rt = build(0x1000, 1, 100);
    let sleeper = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));

    // A resolved wait clears both of these. Here the unit about to park
    // again on the timer fast path still owns them.
    let _ = rt
        .syscall_responses_mut()
        .insert(sleeper, PendingResponse::ReturnCode { code: 0xABCD });
    let _ = rt.timer_wakes.insert(
        GuestTicks::new(1_000),
        sleeper,
        crate::timer_queue::TimerWakeKind::Sleep,
    );

    let mut args = [0u64; 9];
    args[0] = cellgov_ps3_abi::lv2::syscall::TIMER_USLEEP;
    args[1] = 50;
    rt.dispatch_syscall(
        &cellgov_exec::ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::Syscall,
            consumed_cost: cellgov_time::InstructionCost::new(1),
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: None,
            syscall_args: Some(args),
        },
        sleeper,
    );

    assert_eq!(
        rt.lv2_host().invariant_break_site_count(
            "runtime.dispatch_syscall_timer_park_pending_response_displaced"
        ),
        1,
        "the timer fast path overwrites the pending response like every other park \
         site; a count of 0 means the owed r3 and out-pointer writes vanish unrecorded",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.dispatch_syscall_timer_park_timer_wake_displaced"),
        1,
        "a deadline still live when the unit re-parks means the previous wait never \
         cancelled; a count of 0 leaves that uncancelled path invisible",
    );
}
