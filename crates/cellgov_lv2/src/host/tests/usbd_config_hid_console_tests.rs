//! The usbd, sys_config and hid answers held to the reference-console
//! captures `lv2_usbd_config_hid`, `lv2_config_listener_shapes`,
//! `lv2_config_event_read` and `lv2_config_event_consume`. Each test
//! drives the probe's calls through dispatch and compares the return
//! codes and written words with the capture.
//!
//! The tests compare handle, listener and event ids only where one
//! echoes another: the console mints them from system-wide counters.
//! The device list is the console's own bus, so `sys_usbd_get_device_list`
//! is not compared.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_SERVICE_LISTENER_ONCE, SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    SYS_CONFIG_SERVICE_PADMANAGER, SYS_CONFIG_SERVICE_PADMANAGER2, SYS_CONFIG_SERVICE_USER_LIBPAD,
};
use cellgov_ps3_abi::lv2::{errno, syscall};

use super::*;
use crate::host::test_support::{console_payload, extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::sync_primitives::{EventPayload, EventQueueReceive};
use crate::Lv2Dispatch;

const OUT_PTR: u32 = 0x100;
const RECORD_PTR: u32 = 0x400;
/// The probes' listener data: `0x01` and then zeroes, 256 bytes.
const DATA_PTR: u32 = 0x800;
/// The probe's product string for `sys_usbd_unregister_ldd`.
const PRODUCT_PTR: u32 = 0xC00;
const UNKNOWN: u32 = 0xDEAD_BEEF;

fn words(probe: &str) -> Vec<u64> {
    console_payload(probe)
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn runtime() -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    let mut data = [0u8; 256];
    data[0] = 0x01;
    for (addr, bytes) in [(DATA_PTR, &data[..]), (PRODUCT_PTR, &b"cgovnone"[..])] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    FakeRuntime::with_memory(mem)
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

fn written_u32(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> u32 {
    let (code, effects) = dispatch(host, rt, request);
    assert_eq!(code, 0, "{request:?}");
    extract_write_u32(&effects[0])
}

/// A queue and a config handle on it, as each probe's setup makes.
fn open(host: &mut Lv2Host, rt: &FakeRuntime) -> (u32, u32) {
    let queue = written_u32(
        host,
        rt,
        Lv2Request::EventQueueCreate {
            id_ptr: OUT_PTR,
            attr_ptr: 0,
            key: 0,
            size: 16,
        },
    );
    let handle = written_u32(
        host,
        rt,
        Lv2Request::ConfigOpen {
            equeue_id: queue,
            out_handle_ptr: OUT_PTR,
        },
    );
    (queue, handle)
}

fn add_listener(
    handle: u32,
    service_id: u64,
    min_verbosity: u64,
    in_ptr: u32,
    size: u64,
    listener_type: u32,
) -> Lv2Request {
    Lv2Request::ConfigAddServiceListener {
        handle,
        service_id,
        min_verbosity,
        in_ptr,
        size,
        listener_type,
        out_listener_ptr: OUT_PTR,
    }
}

fn read_event(handle: u32, event_id: u32, dst_ptr: u32, size: u64) -> Lv2Request {
    Lv2Request::ConfigGetServiceEvent {
        handle,
        event_id,
        dst_ptr,
        size,
    }
}

fn drain(host: &mut Lv2Host, queue: u32) -> Vec<EventPayload> {
    let mut out = Vec::new();
    while let Some(EventQueueReceive::Delivered(p)) = host.state.event_queues.try_receive(queue) {
        out.push(p);
    }
    out
}

fn hid_is_root(pid: u64) -> Lv2Request {
    Lv2Request::Unsupported {
        number: syscall::HID_IS_ROOT,
        args: [pid, 0, 0, 0, 0, 0, 0, 0],
    }
}

#[test]
fn hid_permission_root_is_set_for_the_callers_own_pid_alone_as_on_the_console() {
    let w = words("lv2_usbd_config_hid");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let own = code(&mut host, &rt, Lv2Request::ProcessGetPid);
    assert_eq!(code(&mut host, &rt, hid_is_root(own)), w[1], "own pid");
    assert_eq!(code(&mut host, &rt, hid_is_root(1)), w[2], "pid 1");
}

#[test]
fn usbd_initialize_finalize_and_unregister_answer_as_the_console_does() {
    let w = words("lv2_usbd_config_hid");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let init = |handle_ptr| Lv2Request::UsbdInitialize { handle_ptr };
    let (rc, effects) = dispatch(&mut host, &rt, init(OUT_PTR));
    assert_eq!(rc, w[3], "first initialize");
    let h1 = extract_write_u32(&effects[0]);
    let (rc, effects) = dispatch(&mut host, &rt, init(OUT_PTR));
    assert_eq!(rc, w[5], "second initialize");
    let h2 = extract_write_u32(&effects[0]);
    assert_ne!(h1, h2, "two live handles");
    assert_eq!(code(&mut host, &rt, init(0)), w[7], "null out pointer");

    let unregister = |handle| Lv2Request::UsbdUnregisterLdd {
        handle,
        product_ptr: PRODUCT_PTR,
        product_len: 8,
    };
    assert_eq!(
        code(&mut host, &rt, unregister(h1)),
        w[10],
        "a product never registered"
    );
    assert_eq!(
        code(&mut host, &rt, unregister(UNKNOWN)),
        w[11],
        "an unknown handle"
    );
    assert_eq!(
        code(&mut host, &rt, Lv2Request::UsbdDetectEvent),
        w[12],
        "detect event"
    );
    assert_eq!(w[13], 1, "the console's detect event returned");

    let finalize = |handle| Lv2Request::UsbdFinalize { handle };
    assert_eq!(code(&mut host, &rt, finalize(h2)), w[14], "finalize h2");
    assert_eq!(
        code(&mut host, &rt, finalize(h2)),
        w[15],
        "finalize h2 again"
    );
    assert_eq!(code(&mut host, &rt, finalize(h1)), w[16], "finalize h1");
    assert_eq!(
        code(&mut host, &rt, finalize(UNKNOWN)),
        w[17],
        "finalize an unknown handle"
    );
}

#[test]
fn config_refusals_on_unknown_ids_answer_as_the_console_does() {
    let w = words("lv2_usbd_config_hid");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (_, handle) = open(&mut host, &rt);
    assert_eq!(
        code(
            &mut host,
            &rt,
            Lv2Request::ConfigOpen {
                equeue_id: UNKNOWN,
                out_handle_ptr: OUT_PTR,
            }
        ),
        w[22],
        "open on an unknown queue"
    );
    assert_eq!(
        code(&mut host, &rt, read_event(handle, 0, RECORD_PTR, 41)),
        w[39],
        "event 0, never sent"
    );
    assert_eq!(
        code(&mut host, &rt, read_event(handle, 0xDEAD, RECORD_PTR, 1024)),
        w[49],
        "an unknown event"
    );
    assert_eq!(
        code(&mut host, &rt, read_event(UNKNOWN, 0, RECORD_PTR, 1024)),
        w[50],
        "an unknown handle"
    );
    let remove = Lv2Request::ConfigRemoveServiceListener {
        handle,
        listener: 0xFFFF_FFFF,
    };
    assert_eq!(
        code(&mut host, &rt, remove),
        w[51],
        "remove an unknown listener"
    );
    assert_eq!(code(&mut host, &rt, remove), w[52], "and again");
    let close = Lv2Request::ConfigClose { handle };
    assert_eq!(code(&mut host, &rt, close), w[53], "close");
    assert_eq!(code(&mut host, &rt, close), w[54], "close again");
}

#[test]
fn a_pad_manager_listener_without_data_hears_the_pad_and_reads_the_record_the_console_writes() {
    let w = words("lv2_config_listener_shapes");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (queue, handle) = open(&mut host, &rt);
    let (rc, effects) = dispatch(
        &mut host,
        &rt,
        add_listener(
            handle,
            SYS_CONFIG_SERVICE_PADMANAGER,
            0,
            0,
            0,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
    );
    assert_eq!(rc, w[4], "add");
    let listener = extract_write_u32(&effects[0]);
    let events = drain(&mut host, queue);
    assert!(!events.is_empty() && w[29] > 0, "both heard the pad");
    let ev = events[0];
    assert_eq!(ev.source, w[30], "source");
    assert_eq!(ev.data1, u64::from(handle), "data1 is the handle");
    assert_eq!(w[31], w[3], "the console's data1 is its handle");
    assert_eq!(ev.data2 >> 32, w[32] >> 32, "registered");
    assert_eq!(ev.data3, w[33], "announced length");

    let (rc, effects) = dispatch(
        &mut host,
        &rt,
        read_event(handle, ev.data2 as u32, RECORD_PTR, 1024),
    );
    assert_eq!(rc, w[34], "read");
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("expected a write, got {:?}", effects[0]);
    };
    let record = bytes.bytes();
    assert_eq!(record.len() as u64, w[35], "bytes written");
    let head = |i: usize| u64::from_be_bytes(record[8 * i..8 * i + 8].try_into().unwrap());
    assert_eq!(head(0) >> 32, u64::from(listener), "listener");
    assert_eq!(w[36] >> 32, w[5], "the console's record names its listener");
    assert_eq!(head(0) & 0xFFFF_FFFF, w[36] & 0xFFFF_FFFF, "registered");
    for i in 1..5 {
        assert_eq!(head(i), w[36 + i], "record word {i}");
    }
}

#[test]
fn listener_shapes_without_pad_data_answer_as_the_console_does() {
    let w = words("lv2_config_listener_shapes");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (_, handle) = open(&mut host, &rt);
    assert_eq!(
        code(
            &mut host,
            &rt,
            add_listener(
                UNKNOWN,
                SYS_CONFIG_SERVICE_PADMANAGER,
                0,
                DATA_PTR,
                8,
                SYS_CONFIG_SERVICE_LISTENER_REPEATING,
            )
        ),
        w[22],
        "an unknown handle"
    );
    assert_eq!(
        code(
            &mut host,
            &rt,
            add_listener(
                handle,
                SYS_CONFIG_SERVICE_USER_LIBPAD,
                0,
                DATA_PTR,
                1,
                SYS_CONFIG_SERVICE_LISTENER_REPEATING,
            )
        ),
        w[25],
        "a user service with a data buffer"
    );
}

#[test]
fn a_pad_manager_listener_with_a_data_buffer_is_enomem_on_the_console_and_accepted_here() {
    let w = words("lv2_config_listener_shapes");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (_, handle) = open(&mut host, &rt);
    let shapes = [
        (
            SYS_CONFIG_SERVICE_PADMANAGER,
            0,
            1,
            SYS_CONFIG_SERVICE_LISTENER_ONCE,
        ),
        (
            SYS_CONFIG_SERVICE_PADMANAGER,
            0,
            8,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
        (
            SYS_CONFIG_SERVICE_PADMANAGER,
            1,
            1,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
        (
            SYS_CONFIG_SERVICE_PADMANAGER2,
            0,
            1,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
        (
            SYS_CONFIG_SERVICE_PADMANAGER,
            0,
            256,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
    ];
    for (n, (service, verbosity, size, kind)) in shapes.into_iter().enumerate() {
        let slot = 4 + 3 * (n + 1);
        assert_eq!(
            w[slot],
            u64::from(errno::CELL_ENOMEM),
            "case {}: the console",
            n + 1
        );
        assert_eq!(
            code(
                &mut host,
                &rt,
                add_listener(handle, service, verbosity, DATA_PTR, size, kind)
            ),
            0,
            "case {}: CellGov accepts the buffer the console refuses",
            n + 1
        );
    }
}

#[test]
fn every_event_read_consumes_the_event_as_on_the_console() {
    let w = words("lv2_config_event_consume");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (queue, handle) = open(&mut host, &rt);
    for i in 0..3 {
        let rc = code(
            &mut host,
            &rt,
            add_listener(
                handle,
                SYS_CONFIG_SERVICE_PADMANAGER,
                0,
                0,
                0,
                SYS_CONFIG_SERVICE_LISTENER_REPEATING,
            ),
        );
        assert_eq!(rc, w[2 + i], "listener {i}");
    }
    let events = drain(&mut host, queue);
    assert_eq!(events.len() as u64, w[6], "three events");
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.data3, w[10 + i], "event {i} data3");
    }
    let id = |i: usize| events[i].data2 as u32;
    let d3 = |i: usize| events[i].data3;

    let (rc, effects) = dispatch(&mut host, &rt, read_event(handle, id(0), RECORD_PTR, d3(0)));
    assert_eq!(rc, w[13], "a full read");
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("expected a write, got {:?}", effects[0]);
    };
    assert_eq!(bytes.bytes().len() as u64, w[14], "bytes written");
    let again =
        |host: &mut Lv2Host, i| code(host, &rt, read_event(handle, id(i), RECORD_PTR, d3(i)));
    assert_eq!(again(&mut host, 0), w[15], "after a full read");

    assert_eq!(
        code(
            &mut host,
            &rt,
            read_event(handle, id(1), RECORD_PTR, d3(1) - 1)
        ),
        w[17],
        "one byte short"
    );
    assert_eq!(again(&mut host, 1), w[19], "after a short read");

    assert_eq!(
        code(&mut host, &rt, read_event(handle, id(2), 0, d3(2))),
        w[21],
        "a null destination"
    );
    assert_eq!(again(&mut host, 2), w[23], "after a null destination");
}

#[test]
fn a_short_read_refuses_and_a_full_one_then_finds_nothing_as_on_the_console() {
    let w = words("lv2_config_event_read");
    let rt = runtime();
    let mut host = Lv2Host::new();
    let (queue, handle) = open(&mut host, &rt);
    let rc = code(
        &mut host,
        &rt,
        add_listener(
            handle,
            SYS_CONFIG_SERVICE_PADMANAGER,
            0,
            0,
            0,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        ),
    );
    assert_eq!(rc, w[4], "add");
    let ev = drain(&mut host, queue)[0];
    assert_eq!(ev.data3, w[11], "announced length");
    let (rc, effects) = dispatch(
        &mut host,
        &rt,
        read_event(handle, ev.data2 as u32, RECORD_PTR, ev.data3 - 1),
    );
    assert_eq!(rc, w[16], "one byte short");
    assert_eq!(effects.len() as u64, w[17], "nothing written");
    assert_eq!(
        code(
            &mut host,
            &rt,
            read_event(handle, ev.data2 as u32, RECORD_PTR, ev.data3)
        ),
        w[18],
        "the full read after it"
    );
}
