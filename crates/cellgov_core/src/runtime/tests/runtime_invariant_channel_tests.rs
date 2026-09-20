//! Dispatch and wake paths that report a broken invariant to the host channel.
//!
//! The two displacement tests run in release builds only: a debug build
//! panics inside the table before the report runs.

use super::*;

use std::collections::BTreeMap;

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

fn spu_init(group_id: u32) -> cellgov_lv2::SpuInitState {
    cellgov_lv2::SpuInitState {
        image: cellgov_lv2::SpuLoadImage::Elf(vec![0xAA]),
        entry_pc: 0,
        stack_ptr: 0x3fff0,
        args: [0; 4],
        group_id,
    }
}

#[test]
fn a_refused_spu_image_returns_efault_and_restores_the_group() {
    let mut rt = build(0x1000, 1, 100);
    let caller = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let group_id = rt
        .lv2_host_mut()
        .thread_groups_mut()
        .create(1)
        .expect("a fresh group id exists");
    rt.lv2_host_mut()
        .thread_groups_mut()
        .get_mut(group_id)
        .expect("group was just created")
        .state = cellgov_lv2::GroupState::Running;
    rt.set_spu_factory(|_, _| {
        Err(crate::SpuFactoryError::ImageLoad {
            detail: "SPU ELF entry point is outside local store".to_string(),
        })
    });

    rt.handle_register_spu(caller, BTreeMap::from([(0, spu_init(group_id))]), vec![], 0);

    assert_eq!(
        rt.registry_mut().drain_syscall_return(caller),
        Some(cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into())
    );
    let group = rt
        .lv2_host()
        .thread_groups()
        .get(group_id)
        .expect("failed start preserves the group");
    assert_eq!(group.state, cellgov_lv2::GroupState::Created);
    assert_eq!(group.remaining_unfinished, 0);
    assert_eq!(
        rt.registry().len(),
        1,
        "a refused first slot registers no SPU"
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.spu_image_load_failed"),
        1
    );
}

#[test]
fn a_late_spu_image_refusal_publishes_no_partial_group() {
    let mut rt = build(0x1000, 1, 100);
    let caller = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    let group_id = rt
        .lv2_host_mut()
        .thread_groups_mut()
        .create(2)
        .expect("a fresh group id exists");
    rt.lv2_host_mut()
        .thread_groups_mut()
        .get_mut(group_id)
        .expect("group was just created")
        .state = cellgov_lv2::GroupState::Running;
    let calls = Cell::new(0);
    rt.set_spu_factory(move |id, _| {
        let call = calls.get();
        calls.set(call + 1);
        if call == 0 {
            Ok(Box::new(CountingUnit::new(id, 100)))
        } else {
            Err(crate::SpuFactoryError::ImageLoad {
                detail: "bad second image".to_string(),
            })
        }
    });

    rt.handle_register_spu(
        caller,
        BTreeMap::from([(0, spu_init(group_id)), (1, spu_init(group_id))]),
        vec![],
        0,
    );

    assert_eq!(
        rt.registry_mut().drain_syscall_return(caller),
        Some(cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into())
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(1)),
        Some(UnitStatus::Finished),
        "the already constructed slot stays inert"
    );
    let groups = rt.lv2_host().thread_groups();
    let group = groups
        .get(group_id)
        .expect("failed start preserves the group");
    assert_eq!(group.state, cellgov_lv2::GroupState::Created);
    assert_eq!(group.remaining_unfinished, 0);
    assert!(groups.unit_for_thread(group_id * 256).is_none());
    assert!(groups.unit_for_thread(group_id * 256 + 1).is_none());
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
