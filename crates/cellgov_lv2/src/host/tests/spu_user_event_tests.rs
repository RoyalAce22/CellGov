//! The user events an SPU thread's interrupt mailbox asks for, served
//! in the order the `spu_intr_mbox_to_event_queue` probe sends them and
//! held against the console's answers and event words.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{event, user_event};

use super::*;
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, extract_write_u32, payload_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::sync_primitives::{EventPayload, EventQueueReceive};
use crate::thread_group::MAX_SLOTS_PER_GROUP;

const GROUP: u32 = 1;
const UNIT: UnitId = UnitId::new(7);
const THREAD: u32 = MAX_SLOTS_PER_GROUP * GROUP;
/// The probe's port connected to queue 1, and the port it leaves free.
const CONNECTED_PORT: u32 = 3;
const UNCONNECTED_PORT: u32 = 4;
/// The port the probe asks the group connect for.
const GROUP_PORT: u32 = 10;

/// Group 1 with one initialized thread, not yet started.
/// Ports connect only before the group starts.
fn initialized_group() -> (Lv2Host, FakeRuntime) {
    const IMG_PTR: u32 = 0x300;
    let mut host = Lv2Host::new();
    host.content_store_mut()
        .register(b"/spu.elf", vec![0xAA, 0xBB]);
    let mut mem = GuestMemory::new(0x4000);
    for (addr, bytes) in [
        (0x100u32, &b"/spu.elf\0"[..]),
        (IMG_PTR, &[0, 0, 0, 1, 0, 0, 0, 1][..]),
    ] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    let rt = FakeRuntime::with_memory(mem);
    let requests = [
        Lv2Request::SpuImageOpen {
            img_ptr: IMG_PTR,
            path_ptr: 0x100,
        },
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id: GROUP,
            thread_num: 0,
            img_ptr: IMG_PTR,
            attr_ptr: 0,
            arg_ptr: 0,
        },
    ];
    for request in requests {
        let d = host.dispatch(request, UnitId::new(0), &rt);
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }), "{d:?}");
    }
    (host, rt)
}

fn queue(host: &mut Lv2Host, rt: &FakeRuntime, size: u32) -> u32 {
    match host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x200,
            attr_ptr: 0,
            key: 0,
            size,
        },
        UnitId::new(0),
        rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

fn connect(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) {
    let d = host.dispatch(request, UnitId::new(0), rt);
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }), "{d:?}");
}

/// The probe's two queues, connected as its PPU program connects them,
/// with the group started and its SPU registered.
fn probe_setup() -> (Lv2Host, u32, u32) {
    let (mut host, rt) = initialized_group();
    let queue1 = queue(&mut host, &rt, 2);
    let queue2 = queue(&mut host, &rt, 4);
    connect(
        &mut host,
        &rt,
        Lv2Request::SpuThreadConnectEvent {
            thread_id: THREAD,
            queue_id: queue1,
            event_type: event::THREAD_USER,
            port: CONNECTED_PORT,
        },
    );
    connect(
        &mut host,
        &rt,
        Lv2Request::SpuThreadGroupConnectEventAllThreads {
            group_id: GROUP,
            queue_id: queue2,
            request_mask: 1 << GROUP_PORT,
            port_ptr: 0x600,
        },
    );
    let started = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: GROUP },
        UnitId::new(0),
        &rt,
    );
    assert!(
        matches!(started, Lv2Dispatch::RegisterSpu { code: 0, .. }),
        "{started:?}"
    );
    host.record_spu(UNIT, GROUP, 0).unwrap();
    (host, queue1, queue2)
}

fn word(port: u32, data0: u32) -> u32 {
    port << user_event::PORT_SHIFT | data0
}

fn send(host: &mut Lv2Host, port: u32, data0: u32, data3: u32) -> Option<u32> {
    let served = host
        .send_spu_thread_user_event(UNIT, word(port, data0), Some(data3))
        .expect("a user event");
    assert_eq!(served.wake, None, "no receiver waits on the queue");
    served.answer
}

fn throw(host: &mut Lv2Host, port: u32, data0: u32, data3: u32) {
    let served = host
        .send_spu_thread_user_event(UNIT, user_event::THROW | word(port, data0), Some(data3))
        .expect("a user event");
    assert_eq!(
        served,
        SpuUserEvent {
            answer: None,
            wake: None
        }
    );
}

fn received(host: &mut Lv2Host, queue: u32) -> EventPayload {
    match host.state.event_queues.try_receive(queue) {
        Some(EventQueueReceive::Delivered(ev)) => ev,
        other => panic!("queue {queue:#x} holds no event: {other:?}"),
    }
}

/// The console's seven event words from word `at`:
///
/// - source, high then low;
/// - 1 when data1 is the thread id, else 0;
/// - data2, high then low;
/// - data3, high then low.
fn console_event(payload: &[u8], at: usize) -> [u32; 7] {
    std::array::from_fn(|i| payload_u32(payload, 4 * (at + i)))
}

fn cellgov_event(ev: &EventPayload) -> [u32; 7] {
    [
        (ev.source >> 32) as u32,
        ev.source as u32,
        u32::from(ev.data1 == u64::from(THREAD)),
        (ev.data2 >> 32) as u32,
        ev.data2 as u32,
        (ev.data3 >> 32) as u32,
        ev.data3 as u32,
    ]
}

#[test]
fn the_probe_sequence_answers_and_delivers_as_the_console_does() {
    let console = console_payload("spu_intr_mbox_to_event_queue");
    assert_eq!(payload_u32(&console, 0), 0, "the probe ran every step");
    assert_eq!(payload_u32(&console, 8), GROUP_PORT, "the group port");

    let (mut host, queue1, queue2) = probe_setup();
    let answers = [
        send(&mut host, CONNECTED_PORT, 0x12_3456, 0xCAFE_BABE),
        {
            throw(&mut host, CONNECTED_PORT, 0x65_4321, 0xDEAD_BEEF);
            send(&mut host, CONNECTED_PORT, 0x11_1111, 0x2222_2222)
        },
        {
            throw(&mut host, CONNECTED_PORT, 0x33_3333, 0x4444_4444);
            send(&mut host, UNCONNECTED_PORT, 0x55_5555, 0x6666_6666)
        },
        {
            throw(&mut host, UNCONNECTED_PORT, 0x77_7777, 0x8888_8888);
            send(&mut host, GROUP_PORT, 0x99_9999, 0xAAAA_AAAA)
        },
    ];
    let console_answers: Vec<Option<u32>> =
        (5..9).map(|w| Some(payload_u32(&console, 4 * w))).collect();
    assert_eq!(answers.to_vec(), console_answers, "OK, EBUSY, ENOTCONN, OK");

    for at in [12, 20] {
        assert_eq!(
            payload_u32(&console, 4 * at),
            0,
            "receive {at} took an event"
        );
        let ev = received(&mut host, queue1);
        assert_eq!(
            cellgov_event(&ev),
            console_event(&console, at + 1),
            "event {at}"
        );
    }
    assert_ne!(
        payload_u32(&console, 4 * 28),
        0,
        "the console's queue 1 is empty"
    );
    assert!(
        !matches!(
            host.state.event_queues.try_receive(queue1),
            Some(EventQueueReceive::Delivered(_))
        ),
        "the send and the throw to the full queue left no event"
    );

    let ev = received(&mut host, queue2);
    let console_group: Vec<u32> = (30..34).map(|w| payload_u32(&console, 4 * w)).collect();
    assert_eq!(
        vec![
            u32::from(ev.data1 == u64::from(THREAD)),
            (ev.data2 >> 32) as u32,
            ev.data2 as u32,
            ev.data3 as u32,
        ],
        console_group,
        "the group port's event"
    );
    assert_eq!(ev.source, event::THREAD_USER_KEY);
}

#[test]
fn a_word_for_another_service_is_refused_by_name() {
    let (mut host, _, queue2) = probe_setup();
    let word = user_event::OTHER_SERVICE | word(GROUP_PORT, 1);
    assert_eq!(
        host.send_spu_thread_user_event(UNIT, word, None),
        Err(SpuThreadError::UnservedInterruptMailbox(word)),
        "the service check comes before the data check"
    );
    assert!(
        !matches!(
            host.state.event_queues.try_receive(queue2),
            Some(EventQueueReceive::Delivered(_))
        ),
        "the refusal sent nothing"
    );
}

#[test]
fn a_user_event_with_an_empty_outbound_mailbox_is_refused_by_name() {
    let (mut host, _, queue2) = probe_setup();
    let word = word(GROUP_PORT, 1);
    assert_eq!(
        host.send_spu_thread_user_event(UNIT, word, None),
        Err(SpuThreadError::InterruptMailboxWithoutData(word))
    );
    assert!(
        !matches!(
            host.state.event_queues.try_receive(queue2),
            Some(EventQueueReceive::Delivered(_))
        ),
        "the refusal sent nothing"
    );
}
