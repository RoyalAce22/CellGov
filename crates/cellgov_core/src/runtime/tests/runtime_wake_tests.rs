//! Sync-wake and join-wake resolution: happy paths plus debug-assert contract panics.

use super::*;

#[test]
#[should_panic(expected = "non-empty tls_bytes requires non-zero tls_base")]
fn ppu_thread_create_tls_base_zero_with_non_empty_tls_panics() {
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
            extra_args: [0; 7],
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

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "no pending response")]
fn resolve_sync_wakes_with_missing_pending_response_debug_panics() {
    let mut rt = build(16, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    // No insert into syscall_responses: park-side missed recording.
    rt.resolve_sync_wakes_for_test(&[waiter]);
}

/// Release-build counterpart to the debug-panic test above. In
/// release the `debug_assert!(false)` compiles out, but
/// `log_invariant_break` is release-active and must surface the
/// dangerous "unit transitions Runnable without an r3 write" path
/// loudly. Without this witness, a future revert that drops the
/// log line would silently restore the live-wrong-value failure
/// mode in release builds with no test catching it.
#[cfg(not(debug_assertions))]
#[test]
fn resolve_sync_wakes_with_missing_pending_response_logs_in_release() {
    let mut rt = build(16, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let pre_breaks = rt.lv2_host().invariant_break_count();
    rt.resolve_sync_wakes_for_test(&[waiter]);
    assert!(
        rt.lv2_host().invariant_break_count() > pre_breaks,
        "release: log_invariant_break must fire on no-pending-response wake; \
         a counter of {pre_breaks} (unchanged) means the path silently \
         absorbed the bug, exactly the regression this guard catches",
    );
}

#[test]
#[should_panic(expected = "join responses resolve through resolve_join_wakes")]
fn resolve_sync_wakes_with_thread_group_join_is_unreachable() {
    let mut rt = build(16, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter,
        cellgov_lv2::PendingResponse::ThreadGroupJoin {
            group_id: 1,
            code: 0,
            cause_ptr: 0x100,
            status_ptr: 0x104,
            cause: 0,
            status: 0,
        },
    );
    rt.resolve_sync_wakes_for_test(&[waiter]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "waiter count already 0")]
fn lwmutex_wake_with_user_struct_waiter_count_zero_debug_panics() {
    let mut rt = build(0x1000, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter,
        cellgov_lv2::PendingResponse::LwMutexWake {
            // Zero-init memory leaves the waiter slot at base+4 = 0,
            // which underflows the host->guest waiter count.
            mutex_ptr: 0x100,
            caller: 0x0100_0001,
        },
    );
    rt.resolve_sync_wakes_for_test(&[waiter]);
}

#[test]
fn resolve_join_wakes_wakes_every_joiner_on_a_finished_group() {
    use cellgov_lv2::{GroupState, PendingResponse};
    let mut rt = build(0x1000, 1, 100);
    let waiter1 = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let waiter2 = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let spu = cellgov_event::UnitId::new(99);

    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(1).unwrap();
    groups.get_mut(gid).unwrap().state = GroupState::Running;
    groups.record_spu(spu, gid, 0).unwrap();

    for (waiter, cause_ptr, status_ptr, code) in [
        (waiter1, 0x100u32, 0x108u32, 0xAAu64),
        (waiter2, 0x200u32, 0x208u32, 0xBBu64),
    ] {
        rt.registry_mut()
            .set_status_override(waiter, UnitStatus::Blocked);
        let _ = rt.syscall_responses_mut().insert(
            waiter,
            PendingResponse::ThreadGroupJoin {
                group_id: gid,
                code,
                cause_ptr,
                status_ptr,
                cause: 0xDEAD_BEEF,
                status: 0xCAFE_BABE,
            },
        );
    }

    rt.resolve_join_wakes_for_test(spu);

    for (waiter, cause_ptr, status_ptr, code) in [
        (waiter1, 0x100u32, 0x108u32, 0xAAu64),
        (waiter2, 0x200u32, 0x208u32, 0xBBu64),
    ] {
        assert_eq!(
            rt.registry().effective_status(waiter),
            Some(UnitStatus::Runnable),
            "{waiter:?} must transition to Runnable",
        );
        assert_eq!(
            rt.registry_mut().drain_syscall_return(waiter),
            Some(code),
            "{waiter:?} must receive its per-pending code",
        );
        assert_eq!(read_guest_u32_be(&rt, cause_ptr), 0xDEAD_BEEF);
        assert_eq!(read_guest_u32_be(&rt, status_ptr), 0xCAFE_BABE);
    }
}

#[test]
fn resolve_join_wakes_leaves_joiners_on_a_different_group_untouched() {
    use cellgov_lv2::{GroupState, PendingResponse};
    let mut rt = build(0x1000, 1, 100);
    let waiter_match = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let waiter_other = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let spu_match = cellgov_event::UnitId::new(98);
    let spu_other = cellgov_event::UnitId::new(99);

    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid_match = groups.create(1).unwrap();
    let gid_other = groups.create(1).unwrap();
    groups.get_mut(gid_match).unwrap().state = GroupState::Running;
    groups.get_mut(gid_other).unwrap().state = GroupState::Running;
    groups.record_spu(spu_match, gid_match, 0).unwrap();
    groups.record_spu(spu_other, gid_other, 0).unwrap();

    rt.registry_mut()
        .set_status_override(waiter_match, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter_match,
        PendingResponse::ThreadGroupJoin {
            group_id: gid_match,
            code: 0xAA,
            cause_ptr: 0x100,
            status_ptr: 0x108,
            cause: 1,
            status: 2,
        },
    );
    rt.registry_mut()
        .set_status_override(waiter_other, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter_other,
        PendingResponse::ThreadGroupJoin {
            group_id: gid_other,
            code: 0xBB,
            cause_ptr: 0x200,
            status_ptr: 0x208,
            cause: 3,
            status: 4,
        },
    );

    rt.resolve_join_wakes_for_test(spu_match);

    assert_eq!(
        rt.registry().effective_status(waiter_match),
        Some(UnitStatus::Runnable),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(waiter_match),
        Some(0xAA),
    );
    assert_eq!(read_guest_u32_be(&rt, 0x100), 1);
    assert_eq!(read_guest_u32_be(&rt, 0x108), 2);

    // Non-match joiner untouched: still Blocked, no r3, no guest writes.
    assert_eq!(
        rt.registry().effective_status(waiter_other),
        Some(UnitStatus::Blocked),
        "joiner on a different group must not wake",
    );
    assert_eq!(rt.registry_mut().drain_syscall_return(waiter_other), None);
    assert_eq!(read_guest_u32_be(&rt, 0x200), 0);
    assert_eq!(read_guest_u32_be(&rt, 0x208), 0);
}

#[test]
fn lwmutex_wake_raw_syscall_path_writes_no_user_struct_and_increments_holds() {
    let mut rt = build(0x1000, 1, 100);
    let waiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    rt.lv2_host_mut().seed_primary_ppu_thread(
        waiter,
        cellgov_lv2::PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        },
    );
    let tid = rt
        .lv2_host()
        .ppu_thread_id_for_unit(waiter)
        .expect("seeded primary thread has a thread id");
    assert_eq!(rt.lv2_host().lwmutex_holds_for(tid), 0);

    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter,
        cellgov_lv2::PendingResponse::LwMutexWake {
            // mutex_ptr == 0: raw-syscall path with no user-space struct.
            mutex_ptr: 0,
            caller: 0x0100_0001,
        },
    );
    rt.resolve_sync_wakes_for_test(&[waiter]);

    // No writes happened: memory at every addr that the user-struct
    // branch would have touched stays zero.
    let mem = rt.memory().as_bytes();
    assert!(
        mem[..16].iter().all(|&b| b == 0),
        "raw-syscall path must not write user-space struct"
    );
    // Holds counter advanced through the unconditional inc.
    assert_eq!(rt.lv2_host().lwmutex_holds_for(tid), 1);
    assert_eq!(rt.registry_mut().drain_syscall_return(waiter), Some(0));
}
