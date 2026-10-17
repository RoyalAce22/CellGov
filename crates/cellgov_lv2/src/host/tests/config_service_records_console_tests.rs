//! The sys_config service records held to the reference-console capture
//! `lv2_config_service_records`. Each test drives the probe's calls
//! through dispatch and compares the return codes, the events each step
//! queues and the records read back with the capture.
//!
//! The probe zeroes a pad-manager record's bytes from 55 on before it
//! records them, so the tests compare the same span.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR_CAPTURED, SYS_CONFIG_SERVICE_EVENT_HEAD_LEN,
    SYS_CONFIG_SERVICE_LISTENER_ONCE, SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    SYS_CONFIG_SERVICE_PADMANAGER, SYS_CONFIG_SERVICE_PADMANAGER2, SYS_CONFIG_SERVICE_USER_BASE,
    SYS_CONFIG_SERVICE_USER_LIBPAD,
};
use cellgov_ps3_abi::lv2::kernel_id;

use super::*;
use crate::host::test_support::{console_payload, extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::sync_primitives::{EventPayload, EventQueueReceive};
use crate::Lv2Dispatch;

const OUT_PTR: u32 = 0x100;
const RECORD_PTR: u32 = 0x400;
const OWN_USER: u64 = SYS_CONFIG_SERVICE_USER_BASE + 0x4347;
/// Record bytes the probe keeps; it zeroes the rest.
const PAD_KEPT: usize =
    SYS_CONFIG_SERVICE_EVENT_HEAD_LEN + SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR_CAPTURED;

fn words() -> Vec<u64> {
    console_payload("lv2_config_service_records")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn dispatch(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> (u64, Vec<Effect>) {
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("{request:?}: expected Immediate, got {other:?}"),
    }
}

fn code(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> u64 {
    dispatch(host, rt, request).0
}

/// The return code and the id written, or `None` when nothing was.
fn created(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> (u64, Option<u32>) {
    let (code, effects) = dispatch(host, rt, request);
    (code, effects.first().map(extract_write_u32))
}

fn drain(host: &mut Lv2Host, queue: u32) -> Vec<EventPayload> {
    let mut out = Vec::new();
    while let Some(EventQueueReceive::Delivered(p)) = host.state.event_queues.try_receive(queue) {
        out.push(p);
    }
    out
}

/// A host whose kernel-id table holds every slot below `first_id`'s, as
/// the console's process did when the probe's first create ran.
fn host_from(first_id: u64) -> Lv2Host {
    let mut host = Lv2Host::new();
    for _ in 0..kernel_id::slot_of(first_id as u32) {
        host.alloc_id(UnitId::new(0), 0x23).expect("a free slot");
    }
    host
}

fn listener(handle: u32, service_id: u64, listener_type: u32) -> Lv2Request {
    Lv2Request::ConfigAddServiceListener {
        handle,
        service_id,
        min_verbosity: 0,
        in_ptr: 0,
        size: 0,
        listener_type,
        out_listener_ptr: OUT_PTR,
    }
}

/// The probe's setup: a queue and a config handle on it.
fn open(host: &mut Lv2Host, rt: &FakeRuntime, w: &[u64]) -> (u32, u32) {
    let (rc, queue) = created(
        host,
        rt,
        Lv2Request::EventQueueCreate {
            id_ptr: OUT_PTR,
            attr_ptr: 0,
            key: 0,
            size: 64,
        },
    );
    assert_eq!(rc, w[0], "queue create");
    let queue = queue.expect("a queue id");
    assert_eq!(u64::from(queue), w[1], "queue");
    let (rc, handle) = created(
        host,
        rt,
        Lv2Request::ConfigOpen {
            equeue_id: queue,
            out_handle_ptr: OUT_PTR,
        },
    );
    assert_eq!(rc, w[2], "open");
    let handle = handle.expect("a handle");
    assert_eq!(u64::from(handle), w[3], "handle");
    (queue, handle)
}

fn runtime() -> FakeRuntime {
    FakeRuntime::with_memory(GuestMemory::new(0x10000))
}

#[test]
fn the_pad_manager_record_and_its_descriptor_head_match_the_console() {
    let w = words();
    let rt = runtime();
    let mut host = host_from(w[1]);
    let (queue, handle) = open(&mut host, &rt, &w);
    let (rc, pad) = created(
        &mut host,
        &rt,
        listener(
            handle,
            SYS_CONFIG_SERVICE_PADMANAGER,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
    );
    assert_eq!(rc, w[4], "add");
    assert_eq!(pad.map(u64::from), Some(w[5]), "listener");

    let events = drain(&mut host, queue);
    assert_eq!(events.len() as u64, w[7], "one pad");
    let ev = events[0];
    assert_eq!(ev.source, w[8], "source");
    assert_eq!(ev.data1, w[9], "data1");
    assert_eq!(ev.data2 >> 32, w[10] >> 32, "registered");
    assert_eq!(ev.data3, w[11], "announced length");

    let (rc, effects) = dispatch(
        &mut host,
        &rt,
        Lv2Request::ConfigGetServiceEvent {
            handle,
            event_id: ev.data2 as u32,
            dst_ptr: RECORD_PTR,
            size: 1024,
        },
    );
    assert_eq!(rc, w[12], "read");
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("expected a write, got {:?}", effects[0]);
    };
    let mut record = bytes.bytes().to_vec();
    assert_eq!(record.len() as u64, w[13], "bytes written");
    record.resize(56, 0);
    record[PAD_KEPT..].fill(0);
    for i in 0..7 {
        let word = u64::from_be_bytes(record[8 * i..8 * i + 8].try_into().unwrap());
        assert_eq!(word, w[14 + i], "record word {i}");
    }
    assert_eq!(w[25], 0xDEAD_D00D, "the console had no second pad");
}

#[test]
fn removing_the_pad_listener_queues_nothing_as_on_the_console() {
    let w = words();
    let rt = runtime();
    let mut host = host_from(w[1]);
    let (queue, handle) = open(&mut host, &rt, &w);
    let (_, pad) = created(
        &mut host,
        &rt,
        listener(
            handle,
            SYS_CONFIG_SERVICE_PADMANAGER,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
    );
    drain(&mut host, queue);
    let rc = code(
        &mut host,
        &rt,
        Lv2Request::ConfigRemoveServiceListener {
            handle,
            listener: pad.expect("a listener"),
        },
    );
    assert_eq!(rc, w[34], "remove");
    assert_eq!(drain(&mut host, queue).len() as u64, w[36], "events after");
}

/// The probe's no-data listener cases, in order: service and type.
const CASES: [(u64, u32); 6] = [
    (
        SYS_CONFIG_SERVICE_PADMANAGER,
        SYS_CONFIG_SERVICE_LISTENER_ONCE,
    ),
    (
        SYS_CONFIG_SERVICE_PADMANAGER2,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    ),
    (0x10, SYS_CONFIG_SERVICE_LISTENER_REPEATING),
    (0x01, SYS_CONFIG_SERVICE_LISTENER_REPEATING),
    (SYS_CONFIG_SERVICE_PADMANAGER, 2),
    (
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    ),
];

/// Each case's return code, removal code and events queued, through
/// CellGov.
fn run_cases() -> Vec<(u64, u64, u64)> {
    let w = words();
    let rt = runtime();
    let mut host = host_from(w[1]);
    let (queue, handle) = open(&mut host, &rt, &w);
    CASES
        .iter()
        .map(|&(service, kind)| {
            let (rc, id) = created(&mut host, &rt, listener(handle, service, kind));
            let removed = code(
                &mut host,
                &rt,
                Lv2Request::ConfigRemoveServiceListener {
                    handle,
                    listener: id.expect("a listener"),
                },
            );
            (rc, removed, drain(&mut host, queue).len() as u64)
        })
        .collect()
}

#[test]
fn every_no_data_listener_registers_and_the_pad_services_and_type_two_queue_as_on_the_console() {
    let w = words();
    let cases = run_cases();
    for (n, &(rc, removed, events)) in cases.iter().enumerate() {
        let slot = 37 + 4 * n;
        assert_eq!(rc, w[slot], "case {n}: add");
        assert_eq!(removed, w[slot + 2], "case {n}: remove");
        if n != 2 && n != 3 {
            assert_eq!(events, w[slot + 3], "case {n}: events");
        }
    }
}

/// The console holds system services CellGov does not register: one on
/// id 0x10 and thirteen on id 0x01. A listener on either hears them
/// there and nothing here.
#[test]
fn services_0x10_and_0x01_answer_a_listener_on_the_console_and_are_absent_here() {
    let w = words();
    let cases = run_cases();
    assert_eq!(w[37 + 4 * 2 + 3], 1, "the console: 0x10");
    assert_eq!(w[37 + 4 * 3 + 3], 13, "the console: 0x01");
    assert_eq!(cases[2].2, 0, "CellGov: 0x10");
    assert_eq!(cases[3].2, 0, "CellGov: 0x01");
}

#[test]
fn a_user_service_of_the_games_own_notifies_its_own_listener_here_and_not_on_the_console() {
    let w = words();
    let rt = runtime();
    let mut host = host_from(w[1]);
    let (queue, handle) = open(&mut host, &rt, &w);
    let (rc, user_listener) = created(
        &mut host,
        &rt,
        listener(handle, OWN_USER, SYS_CONFIG_SERVICE_LISTENER_REPEATING),
    );
    assert_eq!(rc, w[61], "listener");
    assert_eq!(drain(&mut host, queue).len() as u64, w[63], "nothing yet");

    let (rc, service) = created(
        &mut host,
        &rt,
        Lv2Request::ConfigRegisterService {
            handle,
            service_id: OWN_USER,
            user_id: 0,
            verbosity: 0,
            data_ptr: 0,
            size: 0,
            out_service_ptr: OUT_PTR,
        },
    );
    assert_eq!(rc, w[64], "register");
    assert_eq!(w[67], 0, "the console: no event on register");
    assert_eq!(drain(&mut host, queue).len(), 1, "CellGov: one event");

    let rc = code(
        &mut host,
        &rt,
        Lv2Request::ConfigUnregisterService {
            handle,
            service: service.expect("a service"),
        },
    );
    assert_eq!(rc, w[79], "unregister");
    assert_eq!(w[81], 0, "the console: no event on unregister");
    assert_eq!(drain(&mut host, queue).len(), 1, "CellGov: one event");

    let rc = code(
        &mut host,
        &rt,
        Lv2Request::ConfigRemoveServiceListener {
            handle,
            listener: user_listener.expect("a listener"),
        },
    );
    assert_eq!(rc, w[93], "remove");
    assert_eq!(
        code(&mut host, &rt, Lv2Request::ConfigClose { handle }),
        w[94],
        "close"
    );
}
