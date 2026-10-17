//! A process exit hands back the kernel ids the process held, and drops
//! the objects behind them, unless another process can still reach one.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::kernel_id::{compose, generation_of, slot_of};
use cellgov_ps3_abi::lv2::process::{
    BOOT_PROCESS_PID, SYS_MUTEX_OBJECT, SYS_SEMAPHORE_OBJECT, SYS_TIMER_OBJECT,
};
use cellgov_ps3_abi::lv2::sync::SYS_EVENT_PORT_LOCAL;
use cellgov_ps3_abi::lv2::{errno, syscall};

use crate::host::process::ProcessEntry;
use crate::host::test_support::{primary_attrs, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::sync_primitives::{MutexAcquire, MutexAttrs};
use crate::Lv2Dispatch;

const CHILD: u32 = BOOT_PROCESS_PID + 0x100;
const PARENT_UNIT: UnitId = UnitId::new(0);
const CHILD_UNIT: UnitId = UnitId::new(1);
const OUT: u64 = 0x100;
const QUEUE_KEY: u64 = 0x8000_0000_0000_0042;

/// A host with a boot thread on `PARENT_UNIT` and a child process
/// whose thread runs on `CHILD_UNIT`.
fn host_with_child() -> Lv2Host {
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(PARENT_UNIT, primary_attrs());
    assert!(host.state.processes.insert_child(
        CHILD,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
        },
    ));
    host.ppu_threads_mut()
        .create(CHILD_UNIT, primary_attrs())
        .expect("the child thread");
    host.bind_unit_process(CHILD_UNIT, CHILD);
    host
}

/// One call from `unit`: the return code and the u32 of its first write.
fn sc(host: &mut Lv2Host, unit: UnitId, number: u64, args: [u64; 4]) -> (u64, Option<u32>) {
    let rt = FakeRuntime::new(0x1000);
    let [a, b, c, d] = args;
    match host.dispatch(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit, &rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects.iter().find_map(|e| match e {
                Effect::SharedWriteIntent { bytes, .. } => Some(u32::from_be_bytes(
                    bytes.bytes()[..4].try_into().expect("a u32 write"),
                )),
                _ => None,
            });
            (code, written)
        }
        other => panic!("syscall {number}: expected Immediate, got {other:?}"),
    }
}

/// Create through `number` from `unit` and return the id it writes.
fn create(host: &mut Lv2Host, unit: UnitId, number: u64, args: [u64; 4]) -> u32 {
    let (code, id) = sc(host, unit, number, args);
    assert_eq!(code, 0, "create {number}");
    id.expect("a create writes its id")
}

fn timer(host: &mut Lv2Host, unit: UnitId) -> u32 {
    create(host, unit, syscall::TIMER_CREATE, [OUT, 0, 0, 0])
}

#[test]
fn a_childs_exit_hands_back_its_ids_and_the_parent_reuses_the_slots() {
    let mut host = host_with_child();
    let parent_timer = timer(&mut host, PARENT_UNIT);
    let child_ids = [
        timer(&mut host, CHILD_UNIT),
        create(
            &mut host,
            CHILD_UNIT,
            syscall::RWLOCK_CREATE,
            [OUT, 0, 0, 0],
        ),
        create(
            &mut host,
            CHILD_UNIT,
            syscall::EVENT_QUEUE_CREATE,
            [OUT, 0, 0, 8],
        ),
        create(
            &mut host,
            CHILD_UNIT,
            syscall::EVENT_PORT_CREATE,
            [OUT, SYS_EVENT_PORT_LOCAL, 0, 0],
        ),
    ];
    let [_, _, queue, port] = child_ids;
    let (code, _) = sc(
        &mut host,
        CHILD_UNIT,
        syscall::EVENT_PORT_CONNECT_LOCAL,
        [u64::from(port), u64::from(queue), 0, 0],
    );
    assert_eq!(code, 0, "connect");

    host.mark_process_exited(CHILD, 0);

    for id in child_ids {
        assert!(!host.state.kernel_ids.is_live(id), "{id:#x} still live");
    }
    assert!(host.state.kernel_ids.is_live(parent_timer));
    assert!(host.state.event_queues.lookup(queue).is_none());
    assert!(host.state.event_ports.lookup(port).is_none());
    let (code, _) = sc(
        &mut host,
        PARENT_UNIT,
        syscall::TIMER_DESTROY,
        [u64::from(child_ids[0]), 0, 0, 0],
    );
    assert_eq!(
        code,
        u64::from(errno::CELL_ESRCH),
        "the child's timer is gone"
    );
    assert_eq!(host.observability().process_exit_retained_ids, 0);

    for (n, freed) in child_ids.into_iter().enumerate() {
        assert_eq!(
            timer(&mut host, PARENT_UNIT),
            compose(SYS_TIMER_OBJECT, generation_of(freed) + 1, slot_of(freed)),
            "parent create {n} takes the child's slot one generation up",
        );
    }
}

#[test]
fn a_mutex_a_child_thread_still_holds_goes_with_the_child() {
    let mut host = host_with_child();
    let id = host
        .alloc_id(CHILD_UNIT, SYS_MUTEX_OBJECT)
        .expect("a free slot");
    host.state
        .mutexes
        .create_with_id(
            id,
            MutexAttrs {
                priority_policy: 0,
                recursive: false,
                protocol: 0,
            },
        )
        .expect("the mutex");
    let holder = host
        .ppu_thread_id_for_unit(CHILD_UNIT)
        .expect("the child thread");
    assert_eq!(
        host.state.mutexes.try_acquire(id, holder),
        Some(MutexAcquire::Acquired),
    );

    host.mark_process_exited(CHILD, 0);

    assert!(host.state.mutexes.lookup(id).is_none());
    assert!(!host.state.kernel_ids.is_live(id));
    assert_eq!(host.observability().process_exit_retained_mutex_owners, 0);
    assert_eq!(host.observability().process_exit_retained_ids, 0);
}

#[test]
fn a_live_id_with_no_object_is_released_and_logged() {
    let mut host = host_with_child();
    let id = host
        .alloc_id(CHILD_UNIT, SYS_TIMER_OBJECT)
        .expect("a free slot");

    host.mark_process_exited(CHILD, 0);

    assert!(!host.state.kernel_ids.is_live(id));
    assert_eq!(
        host.invariant_break_site_count("process.exit_id_without_object"),
        1,
    );
}

#[test]
fn a_keyed_queue_outlives_its_creator_and_keeps_its_id() {
    let mut host = host_with_child();
    let queue = create(
        &mut host,
        CHILD_UNIT,
        syscall::EVENT_QUEUE_CREATE,
        [OUT, 0, QUEUE_KEY, 8],
    );

    host.mark_process_exited(CHILD, 0);

    assert!(host.state.kernel_ids.is_live(queue));
    assert!(host.state.event_queues.lookup(queue).is_some());
    assert_eq!(host.observability().process_exit_retained_ids, 1);
}

#[test]
fn a_childs_semaphore_a_parent_thread_waits_on_stays() {
    let mut host = host_with_child();
    let id = host
        .alloc_id(CHILD_UNIT, SYS_SEMAPHORE_OBJECT)
        .expect("a free slot");
    host.state
        .semaphores
        .create_with_id(id, 0, 1)
        .expect("the semaphore");
    let parent = host
        .ppu_thread_id_for_unit(PARENT_UNIT)
        .expect("the boot thread");
    host.state
        .semaphores
        .enqueue_waiter(id, parent)
        .expect("the parent waits");

    host.mark_process_exited(CHILD, 0);

    assert!(host.state.kernel_ids.is_live(id));
    assert!(host.state.semaphores.lookup(id).is_some());
    assert_eq!(host.observability().process_exit_retained_ids, 1);
}
