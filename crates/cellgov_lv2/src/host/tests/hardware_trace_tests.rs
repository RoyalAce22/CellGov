//! Dispatch answers held against console-captured output.
//!
//! Each test here is held against lines of one of
//!
//! - `tests/ps3autotests/tests/lv2/sys_semaphore/sys_semaphore.expected`
//! - `tests/ps3autotests/tests/lv2/sys_event_flag/sys_event_flag.expected`
//! - `tests/ps3autotests/tests/lv2/sys_process/sys_process.expected`
//!
//! Each file holds output a PS3 produced for the call the
//! neighbouring `.cpp` issues. The argument values come from those
//! sources, so a failure here marks a divergence from hardware, not
//! from a chosen model. Where a test asserts more than the capture
//! prints, the comment on that test names the part that is CellGov's
//! own answer.
//!
//! The semaphore and event-flag captures print a raw errno beside
//! each call, which fixes the numeric values as well as the arms:
//!
//! - `0x80010002` EINVAL
//! - `0x80010005` ESRCH
//! - `0x8001000a` EBUSY
//! - `0x8001000d` EFAULT
//!
//! The captures print `0x8001000b` ETIMEDOUT too. The runtime's timer
//! queue expires a timed-out wait, so this dispatcher never answers
//! it and no test here asserts it. The process capture prints no
//! errno at all.
//!
//! What the captures do not settle stays out: no test here claims a
//! gate order. Every line that passes a bad id passes arguments that
//! are otherwise legal, so those lines pin an answer without ordering
//! two gates. Two such arguments recur:
//!
//! - a null `num` to `sys_event_flag_cancel`, documented as "discard
//!   the count"
//! - a zero `val` to `sys_semaphore_post`, where only a negative
//!   `val` is a documented refusal

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::test_support::{
    extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, FakeRuntime,
    VALID_SYNC_ATTR_PTR,
};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;

/// A 24-byte window the fake runtime leaves zeroed.
///
/// It stands in for the `memset(&attr_z, 0x00, sizeof ...)` attribute
/// the `.cpp` sources build.
const ZEROED_SYNC_ATTR_PTR: u32 = 0x900;

/// Id the `.cpp` sources reach through their uninitialized `sem_f` /
/// `id_f` stack slot, which no create ever returned.
const STALE_ID: u32 = 0x5EED_BAD1;

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn spawn_thread(host: &mut Lv2Host, unit: UnitId) {
    host.ppu_threads_mut()
        .create(
            unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
}

// ---------------------------------------------------------------
// sys_semaphore
// ---------------------------------------------------------------

fn semaphore_host(initial: i32, max: i32) -> (Lv2Host, FakeRuntime, UnitId, u32) {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial,
            max,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    (host, rt, src, id)
}

#[test]
fn semaphore_create_rejects_the_four_bound_violations_the_console_rejects() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    // (initial, max) pairs, in capture order: max < 0, max == 0,
    // initial < 0, initial > max.
    for (initial, max) in [(0, -1), (0, 0), (-1, 0), (2, 1)] {
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                initial,
                max,
            },
            src,
            &rt,
        );
        assert_eq!(
            code_of(&r),
            errno::CELL_EINVAL.into(),
            "create(initial={initial}, max={max})"
        );
    }
}

#[test]
fn semaphore_create_faults_on_a_null_id_or_attribute_pointer() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    for (id_ptr, attr_ptr) in [(0, VALID_SYNC_ATTR_PTR), (0x100, 0)] {
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr,
                attr_ptr,
                initial: 0,
                max: 1,
            },
            src,
            &rt,
        );
        assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    }
}

#[test]
fn semaphore_create_rejects_a_zeroed_attribute_block() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: ZEROED_SYNC_ATTR_PTR,
            initial: 0,
            max: 1,
        },
        src,
        &rt,
    );
    assert_eq!(code_of(&r), errno::CELL_EINVAL.into());
}

#[test]
fn semaphore_get_value_faults_on_a_null_out_pointer() {
    let (mut host, rt, src, id) = semaphore_host(0, 1);
    let r = host.dispatch(Lv2Request::SemaphoreGetValue { id, out_ptr: 0 }, src, &rt);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
}

#[test]
fn semaphore_calls_on_an_id_no_create_returned_answer_esrch() {
    let (mut host, rt, src, _id) = semaphore_host(0, 1);
    // Both shapes the capture uses for a bad id: a stale stack value
    // and a zero.
    for bad in [STALE_ID, 0] {
        for request in [
            Lv2Request::SemaphoreGetValue {
                id: bad,
                out_ptr: 0x200,
            },
            Lv2Request::SemaphoreWait {
                id: bad,
                timeout: 0,
            },
            Lv2Request::SemaphoreTryWait { id: bad },
            Lv2Request::SemaphoreDestroy { id: bad },
        ] {
            let r = host.dispatch(request, src, &rt);
            assert_eq!(
                code_of(&r),
                errno::CELL_ESRCH.into(),
                "{request:?} on id {bad:#010x}"
            );
        }
    }
}

#[test]
fn semaphore_trywait_on_an_exhausted_count_answers_ebusy() {
    let (mut host, rt, src, id) = semaphore_host(0, 1);
    let r = host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt);
    assert_eq!(code_of(&r), errno::CELL_EBUSY.into());
}

#[test]
fn semaphore_post_to_an_id_no_create_returned_answers_esrch() {
    // The capture posts val 0 to a stale id and to id 0, and gets
    // ESRCH both times. Only a negative val is a documented refusal,
    // so a zero post is not a second fault. These lines pin the ESRCH
    // answer without ordering the id lookup against the count check.
    let (mut host, rt, src, _id) = semaphore_host(0, 1);
    for bad in [STALE_ID, 0] {
        let r = host.dispatch(Lv2Request::SemaphorePost { id: bad, val: 0 }, src, &rt);
        assert_eq!(code_of(&r), errno::CELL_ESRCH.into());
    }
}

#[test]
fn semaphore_post_separates_a_negative_count_from_one_past_max() {
    let (mut host, rt, src, id) = semaphore_host(0, 1);
    let negative = host.dispatch(Lv2Request::SemaphorePost { id, val: -1 }, src, &rt);
    assert_eq!(code_of(&negative), errno::CELL_EINVAL.into());
    let past_max = host.dispatch(Lv2Request::SemaphorePost { id, val: 2 }, src, &rt);
    assert_eq!(code_of(&past_max), errno::CELL_EBUSY.into());
}

#[test]
fn semaphore_destroy_of_an_already_destroyed_id_answers_esrch() {
    let (mut host, rt, src, id) = semaphore_host(0, 1);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::SemaphoreDestroy { id }, src, &rt)),
        0
    );
    let again = host.dispatch(Lv2Request::SemaphoreDestroy { id }, src, &rt);
    assert_eq!(code_of(&again), errno::CELL_ESRCH.into());
}

#[test]
fn semaphore_wait_and_trywait_each_take_one_from_the_count() {
    // The capture creates at (2, 2) and prints get_value between a
    // wait and a trywait: 0x2, 0x1, 0x0.
    let (mut host, rt, src, id) = semaphore_host(2, 2);
    let read = |host: &mut Lv2Host, rt: &FakeRuntime| -> u32 {
        let r = host.dispatch(
            Lv2Request::SemaphoreGetValue { id, out_ptr: 0x200 },
            src,
            rt,
        );
        match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    };
    assert_eq!(read(&mut host, &rt), 2);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, src, &rt)),
        0
    );
    assert_eq!(read(&mut host, &rt), 1);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt)),
        0
    );
    assert_eq!(read(&mut host, &rt), 0);
}

#[test]
fn semaphore_post_wakes_no_more_waiters_than_its_count() {
    // Capture: three threads wait on an exhausted (0, 2) semaphore,
    // post(1) wakes exactly one, and a later post(2) wakes the other
    // two. get_value reads 0x0 after each, so no wake leaves a
    // leftover.
    let (mut host, rt, src, id) = semaphore_host(0, 2);
    let waiters = [UnitId::new(1), UnitId::new(2), UnitId::new(3)];
    for unit in waiters {
        spawn_thread(&mut host, unit);
        let parked = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, unit, &rt);
        assert!(
            matches!(parked, Lv2Dispatch::Block { .. }),
            "waiter must park, got {parked:?}"
        );
    }

    let one = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, src, &rt);
    match one {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            ..
        } => assert_eq!(woken_unit_ids, vec![waiters[0]]),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);

    let two = host.dispatch(Lv2Request::SemaphorePost { id, val: 2 }, src, &rt);
    match two {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            ..
        } => assert_eq!(woken_unit_ids, vec![waiters[1], waiters[2]]),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
}

#[test]
fn semaphore_post_past_max_wakes_nobody_at_all() {
    // Capture: one thread waits on a (0, 2) semaphore, a second
    // thread posts 30 and tolerates EBUSY, and the waiter never
    // prints its wake line. LV2 refuses the post whole: it wakes none
    // of the waiters that post could have covered.
    let (mut host, rt, src, id) = semaphore_host(0, 2);
    let waiter = UnitId::new(1);
    spawn_thread(&mut host, waiter);
    let parked = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, waiter, &rt);
    assert!(
        matches!(parked, Lv2Dispatch::Block { .. }),
        "waiter must park, got {parked:?}"
    );

    let r = host.dispatch(Lv2Request::SemaphorePost { id, val: 30 }, src, &rt);
    assert_eq!(code_of(&r), errno::CELL_EBUSY.into());
    assert_eq!(host.semaphores().lookup(id).unwrap().waiters().len(), 1);
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
}

// ---------------------------------------------------------------
// sys_event_flag
// ---------------------------------------------------------------

fn event_flag_host(init: u64) -> (Lv2Host, FakeRuntime, UnitId, u32) {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    (host, rt, src, id)
}

fn flag_bits(host: &mut Lv2Host, rt: &FakeRuntime, src: UnitId, id: u32) -> u64 {
    let r = host.dispatch(
        Lv2Request::EventFlagGet {
            id,
            flags_ptr: 0x300,
        },
        src,
        rt,
    );
    match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => match &e[0] {
            cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => {
                let b = bytes.bytes();
                assert_eq!(b.len(), 8);
                u64::from_be_bytes(b.try_into().unwrap())
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn event_flag_create_faults_on_a_null_id_or_attribute_pointer() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    for (id_ptr, attr_ptr) in [(0, VALID_SYNC_ATTR_PTR), (0x100, 0)] {
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr,
                attr_ptr,
                init: 0,
            },
            src,
            &rt,
        );
        assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    }
}

#[test]
fn event_flag_create_rejects_a_zeroed_attribute_block() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: ZEROED_SYNC_ATTR_PTR,
            init: 0,
        },
        src,
        &rt,
    );
    assert_eq!(code_of(&r), errno::CELL_EINVAL.into());
}

#[test]
fn event_flag_wait_rejects_a_mode_that_names_neither_or_both_match_rules() {
    // Each call takes only the modes its own lines carry. The
    // capture's wait lines pass mode 0 and mode AND|OR (0x03). Its
    // trywait lines pass mode 0 and 0xFFFFFFFF: the .cpp swapped that
    // call's bit-pattern and mode arguments, and left the printed
    // label saying AND|OR.
    let (mut host, rt, src, id) = event_flag_host(0);
    for mode in [0, 0x03] {
        let wait = host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 1,
                mode,
                result_ptr: 0x200,
                timeout: 0,
            },
            src,
            &rt,
        );
        assert_eq!(
            code_of(&wait),
            errno::CELL_EINVAL.into(),
            "wait mode {mode:#x}"
        );
    }
    for mode in [0, 0xFFFF_FFFF] {
        let trywait = host.dispatch(
            Lv2Request::EventFlagTryWait {
                id,
                bits: 1,
                mode,
                result_ptr: 0x200,
            },
            src,
            &rt,
        );
        assert_eq!(
            code_of(&trywait),
            errno::CELL_EINVAL.into(),
            "trywait mode {mode:#x}"
        );
    }
}

#[test]
fn event_flag_calls_on_an_id_no_create_returned_answer_esrch() {
    let (mut host, rt, src, _id) = event_flag_host(0);
    for bad in [STALE_ID, 0] {
        for request in [
            Lv2Request::EventFlagWait {
                id: bad,
                bits: 0,
                mode: 0x01,
                result_ptr: 0x200,
                timeout: 0,
            },
            Lv2Request::EventFlagTryWait {
                id: bad,
                bits: 0,
                mode: 0x01,
                result_ptr: 0x200,
            },
            Lv2Request::EventFlagSet { id: bad, bits: 0 },
            Lv2Request::EventFlagClear { id: bad, bits: 0 },
            Lv2Request::EventFlagGet {
                id: bad,
                flags_ptr: 0x300,
            },
            Lv2Request::EventFlagDestroy { id: bad },
        ] {
            let r = host.dispatch(request, src, &rt);
            assert_eq!(
                code_of(&r),
                errno::CELL_ESRCH.into(),
                "{request:?} on id {bad:#010x}"
            );
        }
    }
}

#[test]
fn event_flag_cancel_on_an_unknown_id_answers_esrch_through_a_null_count_pointer() {
    // The capture's cancel error lines pass a null `num` beside the
    // bad id and still print ESRCH. A null count pointer means
    // "discard the count", so the pair orders no gates. It pins two
    // answers: ESRCH, and that a null count pointer never becomes
    // EFAULT. The call stores nothing through that pointer.
    let (mut host, rt, src, _id) = event_flag_host(0);
    for bad in [STALE_ID, 0] {
        let r = host.dispatch(
            Lv2Request::EventFlagCancel {
                id: bad,
                num_ptr: 0,
            },
            src,
            &rt,
        );
        match r {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, errno::CELL_ESRCH.into());
                assert!(effects.is_empty(), "a null count pointer stores nothing");
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }
}

#[test]
fn event_flag_get_faults_on_a_null_out_pointer() {
    let (mut host, rt, src, id) = event_flag_host(0);
    let r = host.dispatch(Lv2Request::EventFlagGet { id, flags_ptr: 0 }, src, &rt);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
}

#[test]
fn event_flag_destroy_of_an_already_destroyed_id_answers_esrch() {
    let (mut host, rt, src, id) = event_flag_host(0);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::EventFlagDestroy { id }, src, &rt)),
        0
    );
    let again = host.dispatch(Lv2Request::EventFlagDestroy { id }, src, &rt);
    assert_eq!(code_of(&again), errno::CELL_ESRCH.into());
}

#[test]
fn event_flag_clear_keeps_the_bits_its_argument_names() {
    // The capture prints the whole sequence: a flag holding 0x1f,
    // cleared with 0xaaaaaaaaaaaaaaaa, reads back 0x0a. That value is
    // 0x1f AND 0xaa..aa, so `sys_event_flag_clear` ANDs the caller's
    // mask into the flag.
    let (mut host, rt, src, id) = event_flag_host(0);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::EventFlagSet { id, bits: 0x1f }, src, &rt)),
        0
    );
    assert_eq!(flag_bits(&mut host, &rt, src, id), 0x1f);

    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::EventFlagClear {
                id,
                bits: 0xAAAA_AAAA_AAAA_AAAA,
            },
            src,
            &rt,
        )),
        0
    );
    assert_eq!(flag_bits(&mut host, &rt, src, id), 0x0a);

    assert_eq!(
        code_of(&host.dispatch(Lv2Request::EventFlagClear { id, bits: 0 }, src, &rt)),
        0
    );
    assert_eq!(flag_bits(&mut host, &rt, src, id), 0);
}

#[test]
fn event_flag_set_ors_its_argument_into_the_pattern() {
    // Capture: the flag reads 0x0. A set with 0xaaaaaaaaaaaaaaab
    // reads back that value, then a clear with 1 reads 0x1.
    let (mut host, rt, src, id) = event_flag_host(0);
    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::EventFlagSet {
                id,
                bits: 0xAAAA_AAAA_AAAA_AAAB,
            },
            src,
            &rt,
        )),
        0
    );
    assert_eq!(
        flag_bits(&mut host, &rt, src, id),
        0xAAAA_AAAA_AAAA_AAAB_u64
    );
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::EventFlagClear { id, bits: 1 }, src, &rt)),
        0
    );
    assert_eq!(flag_bits(&mut host, &rt, src, id), 1);
}

#[test]
fn event_flag_cancel_reports_how_many_waiters_it_drained() {
    // Capture: two threads park in `sys_event_flag_wait` on a bit the
    // flag never gains, two others only trywait, and cancel prints
    // "Woke up 2 threads". The trywait pair never parked, so cancel
    // does not count them. The capture prints only that the second
    // trywait "returned an error", never which one. EBUSY is the
    // documented answer for a pattern that does not match.
    let (mut host, rt, src, id) = event_flag_host(0);
    let parked = [UnitId::new(1), UnitId::new(2)];
    for unit in parked {
        spawn_thread(&mut host, unit);
        let r = host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 1,
                mode: 0x02,
                result_ptr: 0,
                timeout: 0,
            },
            unit,
            &rt,
        );
        assert!(
            matches!(r, Lv2Dispatch::Block { .. }),
            "waiter must park, got {r:?}"
        );
    }
    // The capture's trywait threads pass a null result pointer and
    // get an answer without parking.
    let trywait = host.dispatch(
        Lv2Request::EventFlagTryWait {
            id,
            bits: 1,
            mode: 0x12,
            result_ptr: 0,
        },
        src,
        &rt,
    );
    assert_eq!(code_of(&trywait), errno::CELL_EBUSY.into());

    let cancelled = host.dispatch(Lv2Request::EventFlagCancel { id, num_ptr: 0x400 }, src, &rt);
    match cancelled {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects,
        } => {
            assert_eq!(woken_unit_ids, parked.to_vec());
            assert_eq!(
                response_updates.len(),
                parked.len(),
                "every drained waiter needs a response update"
            );
            for (_, resp) in &response_updates {
                assert!(matches!(
                    resp,
                    PendingResponse::EventFlagCancelWake { result_ptr: 0, .. }
                ));
            }
            let count = effects
                .iter()
                .find_map(|e| match e {
                    cellgov_effects::Effect::SharedWriteIntent { range, .. }
                        if range.start().raw() == 0x400 =>
                    {
                        Some(extract_write_u32(e))
                    }
                    _ => None,
                })
                .expect("cancel writes the drained count through num_ptr");
            assert_eq!(count, 2);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

#[test]
fn event_flag_wait_accepts_a_null_result_pointer() {
    // Both the capture's `eventflagWait` and `eventflagTrywait`
    // helpers pass 0 for the result pointer, and neither faults, so a
    // null result pointer is legal.
    //
    // Only the trywait helper reaches a SATISFIED exit with that null
    // pointer -- it prints "worked fine" -- so this test covers both
    // calls and not the wait alone.
    let (mut host, rt, src, id) = event_flag_host(0b0001);
    for request in [
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: 0x02,
            result_ptr: 0,
            timeout: 0,
        },
        Lv2Request::EventFlagTryWait {
            id,
            bits: 0b0001,
            mode: 0x02,
            result_ptr: 0,
        },
    ] {
        let matched = host.dispatch(request, src, &rt);
        match matched {
            Lv2Dispatch::Immediate { code: 0, effects } => {
                assert!(
                    effects.is_empty(),
                    "{request:?}: a null result pointer stores nothing"
                );
            }
            other => panic!("{request:?}: expected Immediate(0), got {other:?}"),
        }
    }
}

#[test]
fn event_flag_wait_on_an_empty_bit_pattern_matches_without_parking() {
    // The capture's first worker waits under AND on `(1 << 0) - 1`,
    // an empty pattern, against a flag that still reads 0. It runs
    // ahead of the other four and never parks: any flag value
    // satisfies an AND over no bits.
    let (mut host, rt, src, id) = event_flag_host(0);
    let r = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0,
            mode: 0x01,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert_eq!(code_of(&r), 0);
    assert!(host.event_flags().lookup(id).unwrap().waiters().is_empty());
}

// ---------------------------------------------------------------
// sys_process
// ---------------------------------------------------------------

#[test]
fn process_get_sdk_version_answers_the_word_a_retail_console_answers() {
    // The capture prints `0xffffffff`, and its source notes that a
    // retail console answers the all-ones word here. A title that
    // declares no SDK version in its process param leaves the same
    // word, so a host with no version set must report it.
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::ProcessGetSdkVersion {
            pid: 0,
            version_out_ptr: 0x200,
        },
        src,
        &rt,
    );
    match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => assert_eq!(extract_write_u32(&e[0]), 0xFFFF_FFFF),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn process_get_paramsfo_answers_the_sixty_four_bytes_a_console_answers() {
    // The capture prints the buffer byte by byte for a title with no
    // PARAM.SFO: 0x01 at 0, 0x04 at 23, 0x01 at 31, and zero
    // everywhere else.
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::ProcessGetParamsfo { buf_ptr: 0x200 }, src, &rt);
    let bytes = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => match &e[0] {
            cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => bytes.bytes().to_vec(),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let mut expected = [0u8; 64];
    expected[0] = 0x01;
    expected[23] = 0x04;
    expected[31] = 0x01;
    assert_eq!(bytes, expected);
}

#[test]
fn process_object_counts_rise_on_create_and_fall_on_destroy() {
    // The capture creates one object of a class and prints the delta
    // against its own baseline. A class it just created reads 1, and
    // a class it destroyed before the next row reads 0 again. Its
    // semaphore column does that across the rows either side of the
    // destroy. The capture prints the event-flag column only while
    // the flag is live, so the fall to 0 there is CellGov's own
    // answer.
    //
    // The capture's columns are positions in its own object-type
    // list, not numeric LV2 class ids. No class id comes out of it:
    // the two used here come from the ABI crate.
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);

    let count = |host: &mut Lv2Host, rt: &FakeRuntime, class_id: u32| -> u32 {
        let r = host.dispatch(
            Lv2Request::ProcessGetNumberOfObject {
                class_id,
                count_out_ptr: 0x200,
            },
            src,
            rt,
        );
        match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    };

    use cellgov_ps3_abi::lv2::process::{SYS_EVENT_FLAG_OBJECT, SYS_SEMAPHORE_OBJECT};

    assert_eq!(count(&mut host, &rt, SYS_SEMAPHORE_OBJECT), 0);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial: 0,
            max: 1,
        },
        src,
        &rt,
    );
    let sem = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(count(&mut host, &rt, SYS_SEMAPHORE_OBJECT), 1);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::SemaphoreDestroy { id: sem }, src, &rt)),
        0
    );
    assert_eq!(count(&mut host, &rt, SYS_SEMAPHORE_OBJECT), 0);

    assert_eq!(count(&mut host, &rt, SYS_EVENT_FLAG_OBJECT), 0);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        src,
        &rt,
    );
    let flag = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(count(&mut host, &rt, SYS_EVENT_FLAG_OBJECT), 1);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::EventFlagDestroy { id: flag }, src, &rt)),
        0
    );
    assert_eq!(count(&mut host, &rt, SYS_EVENT_FLAG_OBJECT), 0);
}
