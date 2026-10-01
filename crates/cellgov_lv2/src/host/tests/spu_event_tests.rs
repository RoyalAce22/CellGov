//! SPU event-binding dispatch tests: the gate order of each arm, and the
//! state each connect leaves and each disconnect removes.

use super::*;
use crate::host::test_support::{extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::thread_group::MAX_SLOTS_PER_GROUP;
use cellgov_effects::Effect;

/// Thread id of slot `slot` of group 1.
const fn thread(slot: u32) -> u32 {
    MAX_SLOTS_PER_GROUP + slot
}

const RT_BYTES: usize = 0x4000;
const PORT_PTR: u32 = 0x300;

/// Group 1 declares `declared` slots and initializes the first
/// `initialized` of them.
fn host_with_group(declared: u32, initialized: u32) -> Lv2Host {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(RT_BYTES);
    host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: declared,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    for slot in 0..initialized {
        host.thread_groups_mut()
            .initialize_thread(
                1,
                slot,
                crate::image::SpuImageHandle::new(1).unwrap(),
                [0; 4],
            )
            .unwrap();
    }
    host
}

/// A new event queue's id.
fn queue(host: &mut Lv2Host) -> u32 {
    let rt = FakeRuntime::new(RT_BYTES);
    match host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x200,
            attr_ptr: 0,
            key: 0,
            size: 8,
        },
        UnitId::new(0),
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

fn immediate(dispatch: Lv2Dispatch) -> (u64, Vec<Effect>) {
    match dispatch {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn call(host: &mut Lv2Host, request: Lv2Request) -> u64 {
    let rt = FakeRuntime::new(RT_BYTES);
    immediate(host.dispatch(request, UnitId::new(0), &rt)).0
}

fn group_connect(host: &mut Lv2Host, group_id: u32, queue_id: u32, event_type: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadGroupConnectEvent {
            group_id,
            queue_id,
            event_type,
        },
    )
}

fn group_disconnect(host: &mut Lv2Host, group_id: u32, event_type: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadGroupDisconnectEvent {
            group_id,
            event_type,
        },
    )
}

fn thread_connect(
    host: &mut Lv2Host,
    thread_id: u32,
    queue_id: u32,
    event_type: u32,
    port: u32,
) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadConnectEvent {
            thread_id,
            queue_id,
            event_type,
            port,
        },
    )
}

fn thread_disconnect(host: &mut Lv2Host, thread_id: u32, event_type: u32, port: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadDisconnectEvent {
            thread_id,
            event_type,
            port,
        },
    )
}

fn bind(host: &mut Lv2Host, thread_id: u32, queue_id: u32, queue_number: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadBindQueue {
            thread_id,
            queue_id,
            queue_number,
        },
    )
}

fn unbind(host: &mut Lv2Host, thread_id: u32, queue_number: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadUnbindQueue {
            thread_id,
            queue_number,
        },
    )
}

fn connect_all(
    host: &mut Lv2Host,
    group_id: u32,
    queue_id: u32,
    request_mask: u64,
    port_ptr: u32,
) -> (u64, Vec<Effect>) {
    let rt = FakeRuntime::new(RT_BYTES);
    immediate(host.dispatch(
        Lv2Request::SpuThreadGroupConnectEventAllThreads {
            group_id,
            queue_id,
            request_mask,
            port_ptr,
        },
        UnitId::new(0),
        &rt,
    ))
}

fn disconnect_all(host: &mut Lv2Host, group_id: u32, port: u32) -> u64 {
    call(
        host,
        Lv2Request::SpuThreadGroupDisconnectEventAllThreads { group_id, port },
    )
}

fn port_byte(effects: &[Effect]) -> u8 {
    match effects {
        [Effect::SharedWriteIntent { bytes, .. }] => {
            assert_eq!(bytes.bytes().len(), 1);
            bytes.bytes()[0]
        }
        other => panic!("expected one byte write, got {other:?}"),
    }
}

#[test]
fn a_group_event_connect_checks_the_group_then_the_type_then_busy_then_the_queue() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(
        group_connect(&mut host, 999, q, event::GROUP_RUN),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(group_connect(&mut host, 1, q, 3), errno::CELL_EINVAL.into());
    // The group check beats a bad type, and a bad type beats an
    // unknown queue.
    assert_eq!(
        group_connect(&mut host, 999, q, 3),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        group_connect(&mut host, 1, 999, 3),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(group_connect(&mut host, 1, q, event::GROUP_RUN), 0);
    assert_eq!(
        host.thread_groups().group_event_queue(1, event::GROUP_RUN),
        Some(q)
    );
    // Busy comes before the queue lookup: an unknown queue on the
    // connected type is EBUSY, on a free type ESRCH.
    assert_eq!(
        group_connect(&mut host, 1, 999, event::GROUP_RUN),
        errno::CELL_EBUSY.into()
    );
    assert_eq!(
        group_connect(&mut host, 1, 999, event::GROUP_EXCEPTION),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        host.thread_groups()
            .group_event_queue(1, event::GROUP_EXCEPTION),
        None
    );
}

#[test]
fn a_system_module_connect_is_refused_with_a_named_break() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    let breaks = host.observability().invariant_break_count;
    assert_eq!(
        group_connect(&mut host, 1, q, event::GROUP_SYSTEM_MODULE),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(host.observability().invariant_break_count, breaks + 1);
    assert_eq!(
        host.thread_groups()
            .group_event_queue(1, event::GROUP_SYSTEM_MODULE),
        None
    );
}

#[test]
fn a_group_event_disconnect_removes_the_queue_and_checks_only_the_group() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(group_connect(&mut host, 1, q, event::GROUP_RUN), 0);
    assert_eq!(
        group_disconnect(&mut host, 999, event::GROUP_RUN),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(group_disconnect(&mut host, 1, event::GROUP_RUN), 0);
    assert_eq!(
        host.thread_groups().group_event_queue(1, event::GROUP_RUN),
        None
    );
    assert_eq!(group_disconnect(&mut host, 1, event::GROUP_RUN), 0);
    assert_eq!(group_disconnect(&mut host, 1, 3), 0);
    assert_eq!(group_connect(&mut host, 1, q, event::GROUP_RUN), 0);
}

#[test]
fn a_thread_port_connect_checks_the_ids_then_the_arguments_then_the_port() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(
        thread_connect(&mut host, 999, q, event::THREAD_USER, 0),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        thread_connect(&mut host, thread(0), 999, event::THREAD_USER, 0),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        thread_connect(&mut host, thread(0), q, 2, 0),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(
        thread_connect(
            &mut host,
            thread(0),
            q,
            event::THREAD_USER,
            event::PORT_COUNT
        ),
        errno::CELL_EINVAL.into()
    );
    // An unknown queue with bad arguments is still ESRCH: ids first.
    assert_eq!(
        thread_connect(&mut host, thread(0), 999, 2, event::PORT_COUNT),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        thread_connect(&mut host, thread(0), q, event::THREAD_USER, 63),
        0
    );
    assert_eq!(
        host.thread_groups().thread_port_queue(thread(0), 63),
        Some(q)
    );
    assert_eq!(
        thread_connect(&mut host, thread(0), q, event::THREAD_USER, 63),
        errno::CELL_EISCONN.into()
    );
}

#[test]
fn a_thread_port_disconnect_removes_the_queue_and_refuses_an_unconnected_port() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(
        thread_connect(&mut host, thread(0), q, event::THREAD_USER, 5),
        0
    );
    assert_eq!(
        thread_disconnect(&mut host, 999, event::THREAD_USER, 5),
        errno::CELL_ESRCH.into()
    );
    // The thread check beats bad arguments.
    assert_eq!(
        thread_disconnect(&mut host, 999, 2, event::PORT_COUNT),
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        thread_disconnect(&mut host, thread(0), 2, 5),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(
        thread_disconnect(&mut host, thread(0), event::THREAD_USER, event::PORT_COUNT),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(
        thread_disconnect(&mut host, thread(0), event::THREAD_USER, 6),
        errno::CELL_ENOTCONN.into()
    );
    assert_eq!(
        thread_disconnect(&mut host, thread(0), event::THREAD_USER, 5),
        0
    );
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 5), None);
    assert_eq!(
        thread_disconnect(&mut host, thread(0), event::THREAD_USER, 5),
        errno::CELL_ENOTCONN.into()
    );
}

#[test]
fn a_queue_bind_refuses_a_taken_number_or_queue_and_a_full_thread() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(bind(&mut host, 999, q, 7), errno::CELL_ESRCH.into());
    assert_eq!(bind(&mut host, thread(0), 999, 7), errno::CELL_ESRCH.into());
    assert_eq!(bind(&mut host, thread(0), q, 7), 0);
    assert_eq!(
        host.thread_groups()
            .thread_queue_bindings(thread(0))
            .unwrap()
            .get(&7),
        Some(&q)
    );
    let other = queue(&mut host);
    assert_eq!(
        bind(&mut host, thread(0), other, 7),
        errno::CELL_EBUSY.into()
    );
    assert_eq!(bind(&mut host, thread(0), q, 8), errno::CELL_EBUSY.into());
    for number in 1..event::QUEUE_BINDING_COUNT as u32 {
        let q = queue(&mut host);
        assert_eq!(bind(&mut host, thread(0), q, 100 + number), 0, "{number}");
    }
    let last = queue(&mut host);
    assert_eq!(
        bind(&mut host, thread(0), last, 200),
        errno::CELL_EAGAIN.into()
    );
    assert_eq!(
        host.thread_groups()
            .thread_queue_bindings(thread(0))
            .unwrap()
            .len(),
        event::QUEUE_BINDING_COUNT
    );
}

#[test]
fn a_queue_unbind_removes_the_binding_and_refuses_an_unbound_number() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(bind(&mut host, thread(0), q, 7), 0);
    assert_eq!(unbind(&mut host, 999, 7), errno::CELL_ESRCH.into());
    assert_eq!(unbind(&mut host, thread(0), 8), errno::CELL_ESRCH.into());
    assert_eq!(unbind(&mut host, thread(0), 7), 0);
    assert!(host
        .thread_groups()
        .thread_queue_bindings(thread(0))
        .unwrap()
        .is_empty());
    assert_eq!(unbind(&mut host, thread(0), 7), errno::CELL_ESRCH.into());
    assert_eq!(bind(&mut host, thread(0), q, 7), 0);
}

#[test]
fn an_all_threads_connect_takes_the_lowest_free_requested_port_on_every_thread() {
    let mut host = host_with_group(2, 2);
    let q = queue(&mut host);
    assert_eq!(
        connect_all(&mut host, 1, q, 0, PORT_PTR).0,
        errno::CELL_EINVAL.into()
    );
    // The zero-mask check beats an unknown group.
    assert_eq!(
        connect_all(&mut host, 999, q, 0, PORT_PTR).0,
        errno::CELL_EINVAL.into()
    );
    assert_eq!(
        connect_all(&mut host, 999, q, 1, PORT_PTR).0,
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        connect_all(&mut host, 1, 999, 1, PORT_PTR).0,
        errno::CELL_ESRCH.into()
    );
    // An unknown queue beats the null-pointer check.
    assert_eq!(
        connect_all(&mut host, 1, 999, 1, 0).0,
        errno::CELL_ESRCH.into()
    );
    assert_eq!(
        connect_all(&mut host, 1, q, 1, 0),
        (errno::CELL_EFAULT.into(), vec![])
    );
    // The refused call connected nothing.
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 0), None);
    assert_eq!(host.thread_groups().thread_port_queue(thread(1), 0), None);
    // Port 1 is taken on one thread, so the mask 0b0110 lands on port 2.
    assert_eq!(
        thread_connect(&mut host, thread(1), q, event::THREAD_USER, 1),
        0
    );
    let (code, effects) = connect_all(&mut host, 1, q, 0b0110, PORT_PTR);
    assert_eq!(code, 0);
    assert_eq!(port_byte(&effects), 2);
    assert_eq!(
        host.thread_groups().thread_port_queue(thread(0), 2),
        Some(q)
    );
    assert_eq!(
        host.thread_groups().thread_port_queue(thread(1), 2),
        Some(q)
    );
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 1), None);
    assert_eq!(
        connect_all(&mut host, 1, q, 0b0110, PORT_PTR).0,
        errno::CELL_EISCONN.into()
    );
}

#[test]
fn an_all_threads_connect_needs_every_declared_slot_initialized() {
    let mut host = host_with_group(2, 1);
    let q = queue(&mut host);
    assert_eq!(
        connect_all(&mut host, 1, q, 1, PORT_PTR).0,
        errno::CELL_ESTAT.into()
    );
    // The state check beats the null-pointer check.
    assert_eq!(
        connect_all(&mut host, 1, q, 1, 0).0,
        errno::CELL_ESTAT.into()
    );
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 0), None);
}

#[test]
fn an_all_threads_disconnect_clears_the_port_on_every_thread() {
    let mut host = host_with_group(2, 2);
    let q = queue(&mut host);
    assert_eq!(connect_all(&mut host, 1, q, 1 << 9, PORT_PTR).0, 0);
    assert_eq!(
        disconnect_all(&mut host, 1, event::PORT_COUNT),
        errno::CELL_EINVAL.into()
    );
    // The port check beats an unknown group.
    assert_eq!(
        disconnect_all(&mut host, 999, event::PORT_COUNT),
        errno::CELL_EINVAL.into()
    );
    assert_eq!(disconnect_all(&mut host, 999, 9), errno::CELL_ESRCH.into());
    assert_eq!(disconnect_all(&mut host, 1, 9), 0);
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 9), None);
    assert_eq!(host.thread_groups().thread_port_queue(thread(1), 9), None);
    assert_eq!(disconnect_all(&mut host, 1, 9), 0);
}

#[test]
fn destroying_a_queue_drops_every_spu_connection_to_it() {
    let mut host = host_with_group(1, 1);
    let q = queue(&mut host);
    assert_eq!(group_connect(&mut host, 1, q, event::GROUP_RUN), 0);
    assert_eq!(
        thread_connect(&mut host, thread(0), q, event::THREAD_USER, 3),
        0
    );
    assert_eq!(bind(&mut host, thread(0), q, 7), 0);
    assert_eq!(
        call(&mut host, Lv2Request::EventQueueDestroy { queue_id: q }),
        0
    );
    assert_eq!(
        host.thread_groups().group_event_queue(1, event::GROUP_RUN),
        None
    );
    assert_eq!(host.thread_groups().thread_port_queue(thread(0), 3), None);
    assert!(host
        .thread_groups()
        .thread_queue_bindings(thread(0))
        .unwrap()
        .is_empty());
}
