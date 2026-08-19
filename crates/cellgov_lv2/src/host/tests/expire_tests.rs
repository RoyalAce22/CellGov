//! Timed-wait expiry per primitive: unqueue, ETIMEDOUT delivery,
//! cond mutex re-acquire, and the missing-waiter invariant.

use super::*;
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    create_mutex_host, extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu,
    FakeRuntime, VALID_SYNC_ATTR_PTR,
};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;
use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

fn etimedout() -> u64 {
    cell_errors::CELL_ETIMEDOUT.into()
}

fn child_attrs() -> PpuThreadAttrs {
    PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 0,
        tls_base: 0,
    }
}

fn extract_created_id(dispatch: &crate::dispatch::Lv2Dispatch) -> u32 {
    match dispatch {
        crate::dispatch::Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

/// One-woken-unit ETIMEDOUT shape shared by the simple primitives.
fn assert_timed_out_wake(out: &ExpiredWait, unit: UnitId) {
    assert_eq!(out.woken_unit_ids, vec![unit]);
    assert_eq!(
        out.response_updates,
        vec![(unit, PendingResponse::ReturnCode { code: etimedout() })]
    );
}

#[test]
fn mutex_timeout_unqueues_without_ownership() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner = UnitId::new(0);
    let waiter = UnitId::new(1);
    seed_primary_ppu(&mut host, owner);
    let waiter_tid = host
        .ppu_threads_mut()
        .create(waiter, child_attrs())
        .unwrap();
    let id = create_mutex_host(&mut host, owner, &rt);

    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        owner,
        &rt,
    );
    let block = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 5000,
        },
        waiter,
        &rt,
    );
    assert!(matches!(block, Lv2Dispatch::Block { .. }));

    let out = host.expire_wait(
        Lv2BlockReason::Mutex { id },
        waiter,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, waiter);
    assert!(out.effects.is_empty());
    let entry = host.mutexes().lookup(id).unwrap();
    assert_ne!(
        entry.owner(),
        Some(waiter_tid),
        "timeout must not transfer ownership"
    );
    assert!(
        entry.waiters().is_empty(),
        "expired waiter must leave the list"
    );
    assert_eq!(
        host.observability().wait_timeout_expiries.get("mutex"),
        Some(&1)
    );
}

#[test]
fn semaphore_timeout_unqueues_and_leaves_count_alone() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial: 0,
            max: 8,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    let block = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 5000 }, src, &rt);
    assert!(
        matches!(block, Lv2Dispatch::Block { .. }),
        "finite-timeout wait must park, got {block:?}"
    );

    let out = host.expire_wait(
        Lv2BlockReason::Semaphore { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, src);
    let entry = host.semaphores().lookup(id).unwrap();
    assert!(entry.waiters().is_empty());
    assert_eq!(
        entry.count(),
        0,
        "timeout must not consume or repair a slot"
    );
}

#[test]
fn event_queue_timeout_writes_nothing() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    let block = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x200,
            timeout: 1_000_000,
        },
        src,
        &rt,
    );
    assert!(matches!(block, Lv2Dispatch::Block { .. }));

    let out = host.expire_wait(
        Lv2BlockReason::EventQueue { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, src);
    assert!(
        out.effects.is_empty(),
        "event-queue timeout must not write the sys_event_t"
    );
    assert!(host.event_queues().lookup(id).unwrap().has_no_waiters());
}

#[test]
fn event_flag_timeout_writes_current_pattern() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    // AND-wait on a pattern the current bits cannot satisfy.
    let block = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0x300,
            timeout: 15_000_000,
        },
        src,
        &rt,
    );
    assert!(
        matches!(block, Lv2Dispatch::Block { .. }),
        "finite-timeout wait must park, got {block:?}"
    );

    let out = host.expire_wait(
        Lv2BlockReason::EventFlag { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_eq!(out.woken_unit_ids, vec![src]);
    assert_eq!(
        out.response_updates,
        vec![(src, PendingResponse::ReturnCode { code: etimedout() })]
    );
    // The current pattern (still 0b101) lands at result_ptr.
    assert_eq!(out.effects.len(), 1);
    match &out.effects[0] {
        cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x300);
            assert_eq!(bytes.bytes(), &0b101u64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    assert!(host.event_flags().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn event_flag_timeout_with_null_result_ptr_writes_nothing() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    let block = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0,
            timeout: 15_000_000,
        },
        src,
        &rt,
    );
    assert!(matches!(block, Lv2Dispatch::Block { .. }));

    let out = host.expire_wait(
        Lv2BlockReason::EventFlag { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, src);
    assert!(
        out.effects.is_empty(),
        "a null result pointer must not produce a guest write at address 0"
    );
}

#[test]
fn event_flag_matched_wait_with_null_result_ptr_writes_nothing() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    // OR-wait the current bits satisfy immediately, result ptr null.
    let matched = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b001,
            mode: 0x02,
            result_ptr: 0,
            timeout: 0,
        },
        src,
        &rt,
    );
    match matched {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            assert!(
                effects.is_empty(),
                "a null result pointer must not produce a guest write at address 0"
            );
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

/// The exact non-match exit shape: the named code plus one 8-byte
/// zero store at `ptr`.
fn assert_zero_store(dispatch: &Lv2Dispatch, expect_code: u64, ptr: u32) {
    match dispatch {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(*code, expect_code);
            assert_eq!(effects.len(), 1);
            match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), u64::from(ptr));
                    assert_eq!(bytes.bytes(), &0u64.to_be_bytes());
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn event_flag_error_and_busy_exits_zero_a_nonnull_result_ptr() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);

    // Bad mode: EINVAL stores 0 (and takes precedence over ESRCH --
    // the id here is unknown too).
    let bad_mode = host.dispatch(
        Lv2Request::EventFlagWait {
            id: id + 1,
            bits: 0b1,
            mode: 0x03,
            result_ptr: 0x300,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert_zero_store(&bad_mode, cell_errors::CELL_EINVAL.into(), 0x300);

    // Unknown id: ESRCH stores 0.
    let unknown = host.dispatch(
        Lv2Request::EventFlagTryWait {
            id: id + 1,
            bits: 0b1,
            mode: 0x01,
            result_ptr: 0x308,
        },
        src,
        &rt,
    );
    assert_zero_store(&unknown, cell_errors::CELL_ESRCH.into(), 0x308);

    // Trywait no-match: EBUSY stores 0.
    let busy = host.dispatch(
        Lv2Request::EventFlagTryWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0x310,
        },
        src,
        &rt,
    );
    assert_zero_store(&busy, cell_errors::CELL_EBUSY.into(), 0x310);
}

#[test]
fn event_flag_cancel_reports_the_pattern_through_each_result_ptr() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    let canceller = UnitId::new(1);
    seed_primary_ppu(&mut host, src);
    host.ppu_threads_mut()
        .create(canceller, child_attrs())
        .unwrap();
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    let block = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0x300,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(block, Lv2Dispatch::Block { .. }));

    let cancel = host.dispatch(
        Lv2Request::EventFlagCancel { id, num_ptr: 0x400 },
        canceller,
        &rt,
    );
    match cancel {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects,
        } => {
            assert_eq!(woken_unit_ids, vec![src]);
            assert_eq!(
                response_updates,
                vec![(
                    src,
                    PendingResponse::ReturnCode {
                        code: cell_errors::CELL_ECANCELED.into()
                    }
                )]
            );
            // Pattern write for the cancelled waiter, then the count.
            assert_eq!(effects.len(), 2);
            match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), 0x300);
                    assert_eq!(bytes.bytes(), &0b101u64.to_be_bytes());
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

#[test]
fn event_flag_cancel_esrch_zeroes_a_nonnull_num_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);

    // Unknown id with a non-null num pointer: the count is zeroed
    // before the id lookup, so ESRCH still stores 0 (u32).
    let cancel = host.dispatch(
        Lv2Request::EventFlagCancel {
            id: 999,
            num_ptr: 0x400,
        },
        src,
        &rt,
    );
    match cancel {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, u64::from(cell_errors::CELL_ESRCH));
            assert_eq!(effects.len(), 1);
            match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), 0x400);
                    assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }

    let cancel_null = host.dispatch(
        Lv2Request::EventFlagCancel {
            id: 999,
            num_ptr: 0,
        },
        src,
        &rt,
    );
    match cancel_null {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, u64::from(cell_errors::CELL_ESRCH));
            assert!(
                effects.is_empty(),
                "a null num pointer must not produce a guest write at address 0"
            );
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

// Release-only: the duplicate-park refusal fires a debug_assert in
// EventFlagTable::enqueue_waiter, so the EFAULT exit shape is only
// observable without debug assertions.
#[cfg(not(debug_assertions))]
#[test]
fn a_duplicate_event_flag_park_zeroes_the_result_and_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b101,
        },
        src,
        &rt,
    );
    let id = extract_created_id(&created);
    let block = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0x300,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(block, Lv2Dispatch::Block { .. }));

    // Same thread parks again: every non-park exit stores 0 through
    // a non-null result pointer, the host refusal included.
    let dup = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b010,
            mode: 0x01,
            result_ptr: 0x308,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert_zero_store(&dup, cell_errors::CELL_EFAULT.into(), 0x308);
}

fn cond_setup(host: &mut Lv2Host, rt: &FakeRuntime, owner: UnitId) -> (u32, u32) {
    let mutex_id = create_mutex_host(host, owner, rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x110,
            mutex_id,
            attr_ptr: 0,
        },
        owner,
        rt,
    );
    (extract_created_id(&created), mutex_id)
}

#[test]
fn cond_timeout_reacquires_free_mutex_and_wakes() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let (cond_id, mutex_id) = cond_setup(&mut host, &rt, src);

    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let block = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 1_000_000,
        },
        src,
        &rt,
    );
    assert!(
        matches!(
            block,
            Lv2Dispatch::Block { .. } | Lv2Dispatch::BlockAndWake { .. }
        ),
        "cond wait must park, got {block:?}"
    );
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        None,
        "cond wait must release the mutex"
    );

    let src_tid = host.ppu_threads().thread_id_for_unit(src).unwrap();
    let out = host.expire_wait(
        Lv2BlockReason::Cond {
            id: cond_id,
            mutex_id,
            mutex_kind: CondMutexKind::Mutex,
        },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, src);
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(src_tid),
        "cond timeout must return holding the mutex"
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_timeout_with_contended_mutex_reparks_untimed() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let sleeper = UnitId::new(0);
    let holder = UnitId::new(1);
    seed_primary_ppu(&mut host, sleeper);
    let holder_tid = host
        .ppu_threads_mut()
        .create(holder, child_attrs())
        .unwrap();
    let (cond_id, mutex_id) = cond_setup(&mut host, &rt, sleeper);

    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        sleeper,
        &rt,
    );
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 1_000_000,
        },
        sleeper,
        &rt,
    );
    // The peer takes the freed mutex before the timeout fires.
    let acquired = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        holder,
        &rt,
    );
    assert!(matches!(acquired, Lv2Dispatch::Immediate { code: 0, .. }));

    let sleeper_tid = host.ppu_threads().thread_id_for_unit(sleeper).unwrap();
    let out = host.expire_wait(
        Lv2BlockReason::Cond {
            id: cond_id,
            mutex_id,
            mutex_kind: CondMutexKind::Mutex,
        },
        sleeper,
        cellgov_time::GuestTicks::ZERO,
    );
    // ETIMEDOUT is staged but the unit is NOT woken: it waits
    // untimed on the mutex and returns holding it after the unlock.
    assert!(out.woken_unit_ids.is_empty());
    assert_eq!(
        out.response_updates,
        vec![(sleeper, PendingResponse::ReturnCode { code: etimedout() })]
    );
    let entry = host.mutexes().lookup(mutex_id).unwrap();
    assert_eq!(entry.owner(), Some(holder_tid));
    assert!(entry.waiters().contains(sleeper_tid));

    // The unlock now transfers ownership and wakes the sleeper
    // through the ordinary sync-wake path. Its response_updates must
    // stay empty: an override here would clobber the staged ETIMEDOUT.
    let unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id }, holder, &rt);
    match unlock {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![sleeper]);
            assert!(
                response_updates.is_empty(),
                "unlock must not restage the sleeper's response; the staged \
                 ETIMEDOUT is what resolves at wake"
            );
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(sleeper_tid)
    );
}

#[test]
fn a_cond_park_that_transfers_the_mutex_still_counts_its_timeout() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let sleeper = UnitId::new(0);
    let peer = UnitId::new(1);
    seed_primary_ppu(&mut host, sleeper);
    host.ppu_threads_mut().create(peer, child_attrs()).unwrap();
    let (cond_id, mutex_id) = cond_setup(&mut host, &rt, sleeper);

    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        sleeper,
        &rt,
    );
    // Peer parks on the held mutex so the cond wait's release
    // transfers ownership, taking the BlockAndWake shape.
    let peer_block = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        peer,
        &rt,
    );
    assert!(matches!(peer_block, Lv2Dispatch::Block { .. }));
    let block = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 7_000,
        },
        sleeper,
        &rt,
    );
    assert!(
        matches!(block, Lv2Dispatch::BlockAndWake { .. }),
        "release must transfer to the parked peer, got {block:?}"
    );
    // The runtime registers a wake deadline for BlockAndWake parks
    // exactly as for Block parks; the witness must see both.
    assert_eq!(
        host.observability().park_timeouts.get(&("CondWait", 7_000)),
        Some(&1),
        "a timed BlockAndWake park must land in park_timeouts"
    );
}

#[test]
fn lwmutex_timeout_unqueues_without_guest_writes() {
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = host.lwmutexes_mut().create().unwrap();
    let tid = host.ppu_threads().thread_id_for_unit(src).unwrap();
    host.lwmutexes_mut().enqueue_waiter(id, tid).unwrap();

    let out = host.expire_wait(
        Lv2BlockReason::LwMutex { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_timed_out_wake(&out, src);
    assert!(
        out.effects.is_empty(),
        "lwmutex timeout must not touch the guest sys_lwmutex_t"
    );
    assert!(host.lwmutexes().lookup(id).unwrap().waiters().is_empty());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "expire_wait.missing_waiter")]
fn expiry_without_waiter_record_is_an_invariant_break() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = create_mutex_host(&mut host, src, &rt);
    // Never parked: the cancel invariant says this cannot happen,
    // so the expiry is refused loudly.
    host.expire_wait(
        Lv2BlockReason::Mutex { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn expiry_without_waiter_record_returns_empty_in_release() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = create_mutex_host(&mut host, src, &rt);
    let out = host.expire_wait(
        Lv2BlockReason::Mutex { id },
        src,
        cellgov_time::GuestTicks::ZERO,
    );
    assert_eq!(out, ExpiredWait::default());
    assert!(host.observability().invariant_break_count > 0);
}
