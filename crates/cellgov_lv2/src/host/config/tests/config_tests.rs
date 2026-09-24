//! `sys_config` dispatch tests: handle lifecycle, pad-manager replay filters, record layout, and receiver wake.

use super::*;
use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;
use crate::sync_primitives::EventPayload;
use crate::sync_primitives::EventQueueReceive;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_EVENT_SOURCE_SERVICE, SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR,
    SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN, SYS_CONFIG_SERVICE_EVENT_HEAD_LEN,
    SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN, SYS_CONFIG_SERVICE_LISTENER_ONCE,
    SYS_CONFIG_SERVICE_PADMANAGER, SYS_CONFIG_SERVICE_PADMANAGER2,
};
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_SERVICE_LISTENER_REPEATING, SYS_CONFIG_SERVICE_USER_LIBPAD,
};
use cellgov_ps3_abi::lv2::errno;

const HANDLE_PTR: u32 = 0x104;
const LISTENER_PTR: u32 = 0x108;
const SERVICE_PTR: u32 = 0x10c;
const RECORD_PTR: u32 = 0x400;
/// Listener buffer whose first byte opens the pad-manager filter.
const PAD_FILTER_PTR: u32 = 0x200;
/// Listener buffer that leaves the filter closed.
const NO_FILTER_PTR: u32 = 0x210;
/// Four bytes of user-service data.
const USER_DATA_PTR: u32 = 0x220;
const USER_DATA: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

fn src() -> UnitId {
    UnitId::new(0)
}

fn runtime() -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    for (addr, bytes) in [
        (PAD_FILTER_PTR, &[0x01u8, 0, 0, 0][..]),
        (NO_FILTER_PTR, &[0x00u8, 0, 0, 0][..]),
        (USER_DATA_PTR, &USER_DATA[..]),
    ] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    FakeRuntime::with_memory(mem)
}

fn written_u32(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0) with a write, got {other:?}"),
    }
}

fn written_bytes(d: &Lv2Dispatch) -> Vec<u8> {
    match d {
        Lv2Dispatch::Immediate { code: 0, effects } => match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => bytes.bytes().to_vec(),
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        other => panic!("expected Immediate(0) with a write, got {other:?}"),
    }
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn create_queue(host: &mut Lv2Host, rt: &FakeRuntime, size: u32) -> u32 {
    written_u32(&host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size,
        },
        src(),
        rt,
    ))
}

fn open(host: &mut Lv2Host, rt: &FakeRuntime, queue: u32) -> u32 {
    written_u32(&host.dispatch(
        Lv2Request::ConfigOpen {
            equeue_id: queue,
            out_handle_ptr: HANDLE_PTR,
        },
        src(),
        rt,
    ))
}

fn add_listener(
    host: &mut Lv2Host,
    rt: &FakeRuntime,
    handle: u32,
    service_id: u64,
    in_ptr: u32,
    size: u64,
    listener_type: u32,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::ConfigAddServiceListener {
            handle,
            service_id,
            min_verbosity: 0,
            in_ptr,
            size,
            listener_type,
            out_listener_ptr: LISTENER_PTR,
        },
        src(),
        rt,
    )
}

fn register_user_service(host: &mut Lv2Host, rt: &FakeRuntime, handle: u32) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::ConfigRegisterService {
            handle,
            service_id: SYS_CONFIG_SERVICE_USER_LIBPAD,
            user_id: 7,
            verbosity: 3,
            data_ptr: USER_DATA_PTR,
            size: USER_DATA.len() as u64,
            out_service_ptr: SERVICE_PTR,
        },
        src(),
        rt,
    )
}

fn drain(host: &mut Lv2Host, queue: u32) -> Vec<EventPayload> {
    let mut out = Vec::new();
    while let Some(EventQueueReceive::Delivered(p)) = host.state.event_queues.try_receive(queue) {
        out.push(p);
    }
    out
}

fn get_record(
    host: &mut Lv2Host,
    rt: &FakeRuntime,
    handle: u32,
    event_id: u32,
    size: u64,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::ConfigGetServiceEvent {
            handle,
            event_id,
            dst_ptr: RECORD_PTR,
            size,
        },
        src(),
        rt,
    )
}

#[test]
fn open_on_an_unknown_queue_is_esrch() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let d = host.dispatch(
        Lv2Request::ConfigOpen {
            equeue_id: 0x1234,
            out_handle_ptr: HANDLE_PTR,
        },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
    assert!(host.state.config.is_pristine());
}

#[test]
fn open_writes_a_handle_and_seeds_both_pad_manager_services() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    assert_eq!(host.state.config.handle(handle).unwrap().queue_id, queue);
    assert!(host.state.config.seeded());
    assert_eq!(host.state.config.service_count(), 2);
    assert!(drain(&mut host, queue).is_empty(), "no listener, no event");
}

#[test]
fn a_second_open_does_not_seed_again() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let first = open(&mut host, &rt, queue);
    let second = open(&mut host, &rt, queue);
    assert_ne!(first, second);
    assert_eq!(host.state.config.service_count(), 2);
}

#[test]
fn a_pad_manager_listener_leading_with_01_is_replayed_the_seeded_service() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let d = add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER,
        PAD_FILTER_PTR,
        4,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let listener = written_u32(&d);
    let events = drain(&mut host, queue);
    assert_eq!(events.len(), 1);
    let ev = events[0];
    assert_eq!(ev.source, SYS_CONFIG_EVENT_SOURCE_SERVICE);
    assert_eq!(ev.data1, u64::from(handle));
    assert_eq!(ev.data2 >> 32, 1, "registered");
    assert_eq!(ev.data2 as u32, 0, "first event id");
    assert_eq!(
        ev.data3,
        (SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN + SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR.len())
            as u64,
        "data3 is the oracle's sizeof-based size, not the bytes written"
    );
    assert_eq!(host.state.config.event(0).unwrap().listener, listener);
}

#[test]
fn a_pad_manager_listener_without_the_leading_byte_hears_nothing() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    for (ptr, size) in [(NO_FILTER_PTR, 4u64), (PAD_FILTER_PTR, 0)] {
        let d = add_listener(
            &mut host,
            &rt,
            handle,
            SYS_CONFIG_SERVICE_PADMANAGER,
            ptr,
            size,
            SYS_CONFIG_SERVICE_LISTENER_REPEATING,
        );
        assert_eq!(code_of(&d), 0);
    }
    assert!(drain(&mut host, queue).is_empty());
    assert_eq!(host.state.config.event_count(), 0);
}

#[test]
fn a_pad_manager2_listener_needs_no_data() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER2,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    assert_eq!(drain(&mut host, queue).len(), 1);
}

#[test]
fn a_min_verbosity_above_the_service_filters_it_out() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let d = host.dispatch(
        Lv2Request::ConfigAddServiceListener {
            handle,
            service_id: SYS_CONFIG_SERVICE_PADMANAGER2,
            min_verbosity: 2,
            in_ptr: 0,
            size: 0,
            listener_type: SYS_CONFIG_SERVICE_LISTENER_REPEATING,
            out_listener_ptr: LISTENER_PTR,
        },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), 0);
    assert!(drain(&mut host, queue).is_empty());
}

#[test]
fn get_service_event_writes_the_record() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let listener = written_u32(&add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER,
        PAD_FILTER_PTR,
        4,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    ));
    let ev = drain(&mut host, queue)[0];
    let record = written_bytes(&get_record(
        &mut host,
        &rt,
        handle,
        ev.data2 as u32,
        ev.data3,
    ));
    assert_eq!(
        record.len(),
        SYS_CONFIG_SERVICE_EVENT_HEAD_LEN + SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR.len()
    );
    assert_eq!(
        ev.data3,
        record.len() as u64 + 7,
        "announced size carries the oracle struct's tail padding"
    );
    assert_eq!(&record[0..4], &listener.to_be_bytes());
    assert_eq!(&record[4..8], &1u32.to_be_bytes(), "registered");
    assert_eq!(&record[8..16], &SYS_CONFIG_SERVICE_PADMANAGER.to_be_bytes());
    assert_eq!(&record[16..24], &0u64.to_be_bytes(), "user_id = port 0");
    assert_eq!(&record[24..32], &1u64.to_be_bytes(), "verbosity");
    assert_eq!(&record[32..36], &26u32.to_be_bytes(), "data_size");
    assert_eq!(&record[36..40], &0u32.to_be_bytes(), "padding");
    assert_eq!(&record[40..], &SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR);
}

#[test]
fn get_service_event_with_a_short_buffer_is_eagain() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER2,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let ev = drain(&mut host, queue)[0];
    let d = get_record(&mut host, &rt, handle, ev.data2 as u32, ev.data3 - 1);
    assert_eq!(code_of(&d), u64::from(errno::CELL_EAGAIN));
    let written =
        (SYS_CONFIG_SERVICE_EVENT_HEAD_LEN + SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR.len()) as u64;
    let d = get_record(&mut host, &rt, handle, ev.data2 as u32, written);
    assert_eq!(
        code_of(&d),
        u64::from(errno::CELL_EAGAIN),
        "a buffer sized to the bytes written is still below the announced floor"
    );
    let d = get_record(&mut host, &rt, handle, ev.data2 as u32, ev.data3);
    assert_eq!(code_of(&d), 0, "exactly data3 bytes suffice");
}

#[test]
fn a_registration_record_read_after_unregister_reports_the_live_state() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let service = written_u32(&register_user_service(&mut host, &rt, handle));
    let registered = drain(&mut host, queue)[0];
    assert_eq!(registered.data2 >> 32, 1);
    host.dispatch(
        Lv2Request::ConfigUnregisterService { handle, service },
        src(),
        &rt,
    );
    assert_eq!(drain(&mut host, queue).len(), 1, "the unregister event");
    let record = written_bytes(&get_record(
        &mut host,
        &rt,
        handle,
        registered.data2 as u32,
        registered.data3,
    ));
    assert_eq!(
        record.len(),
        SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN,
        "the record follows the service, not what data2 announced"
    );
    assert_eq!(&record[4..8], &0u32.to_be_bytes(), "registered = 0 now");
    assert_eq!(
        &record[16..24],
        &7u64.to_be_bytes(),
        "user_id still readable"
    );
    assert!(
        host.state
            .config
            .event(registered.data2 as u32)
            .unwrap()
            .registered,
        "what the queue announced is kept as announced"
    );
}

#[test]
fn a_listener_added_after_unregister_is_not_replayed_the_held_service() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let service = written_u32(&register_user_service(&mut host, &rt, handle));
    host.dispatch(
        Lv2Request::ConfigUnregisterService { handle, service },
        src(),
        &rt,
    );
    assert_eq!(drain(&mut host, queue).len(), 2);
    assert!(
        host.state.config.service(service).is_some(),
        "the first listener's events keep the service alive"
    );
    let d = add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    assert_eq!(code_of(&d), 0);
    assert!(
        drain(&mut host, queue).is_empty(),
        "an unregistered service is not in the replay set"
    );
    assert_eq!(
        host.state.config.event_count(),
        2,
        "no event minted for the second listener"
    );
}

#[test]
fn get_service_event_through_another_handle_is_esrch() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let other = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER2,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let ev = drain(&mut host, queue)[0];
    let d = get_record(&mut host, &rt, other, ev.data2 as u32, ev.data3);
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
    let d = get_record(&mut host, &rt, handle, 99, ev.data3);
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
}

#[test]
fn a_parked_receiver_wakes_with_the_service_event() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    seed_primary_ppu(&mut host, src());
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let parked = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: queue,
            out_ptr: 0x300,
            timeout: 0,
        },
        src(),
        &rt,
    );
    assert!(
        matches!(parked, Lv2Dispatch::Block { .. }),
        "got {parked:?}"
    );
    let d = host.dispatch(
        Lv2Request::ConfigAddServiceListener {
            handle,
            service_id: SYS_CONFIG_SERVICE_PADMANAGER2,
            min_verbosity: 0,
            in_ptr: 0,
            size: 0,
            listener_type: SYS_CONFIG_SERVICE_LISTENER_REPEATING,
            out_listener_ptr: LISTENER_PTR,
        },
        UnitId::new(1),
        &rt,
    );
    match d {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects,
        } => {
            assert_eq!(woken_unit_ids, vec![src()]);
            assert_eq!(effects.len(), 1, "the listener id write");
            assert_eq!(response_updates.len(), 1);
            match response_updates[0] {
                (
                    unit,
                    PendingResponse::EventQueueReceive {
                        out_ptr: 0x300,
                        payload: Some(p),
                    },
                ) => {
                    assert_eq!(unit, src());
                    assert_eq!(p.source, SYS_CONFIG_EVENT_SOURCE_SERVICE);
                    assert_eq!(p.data1, u64::from(handle));
                }
                other => panic!("unexpected response update {other:?}"),
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert!(
        drain(&mut host, queue).is_empty(),
        "handed off, not buffered"
    );
}

#[test]
fn a_once_listener_hears_one_registration_and_a_repeating_one_hears_each() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_ONCE,
    );
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    assert!(drain(&mut host, queue).is_empty(), "nothing registered yet");
    let first = written_u32(&register_user_service(&mut host, &rt, handle));
    assert_eq!(
        drain(&mut host, queue).len(),
        2,
        "both listeners hear the first"
    );
    let second = written_u32(&register_user_service(&mut host, &rt, handle));
    assert_ne!(first, second);
    assert_eq!(
        drain(&mut host, queue).len(),
        1,
        "only the repeating listener hears the second"
    );
    assert_eq!(host.state.config.service_count(), 4, "two seeded, two user");
}

#[test]
fn unregister_notifies_with_registered_zero_and_writes_the_short_record() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let listener = written_u32(&add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    ));
    let service = written_u32(&register_user_service(&mut host, &rt, handle));
    let registered = drain(&mut host, queue)[0];
    let d = host.dispatch(
        Lv2Request::ConfigUnregisterService { handle, service },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), 0);
    let gone = drain(&mut host, queue)[0];
    assert_eq!(gone.data2 >> 32, 0, "registered = 0");
    assert_ne!(gone.data2 as u32, registered.data2 as u32);
    assert_eq!(
        gone.data3, registered.data3,
        "size announced as for a live service"
    );
    let record = written_bytes(&get_record(
        &mut host,
        &rt,
        handle,
        gone.data2 as u32,
        gone.data3,
    ));
    assert_eq!(record.len(), SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN);
    assert_eq!(&record[0..4], &listener.to_be_bytes());
    assert_eq!(&record[4..8], &0u32.to_be_bytes());
    assert_eq!(
        &record[8..16],
        &SYS_CONFIG_SERVICE_USER_LIBPAD.to_be_bytes()
    );
    assert_eq!(&record[16..24], &7u64.to_be_bytes());
    assert!(
        host.state.config.service(service).is_some(),
        "kept while its events are readable"
    );
    let again = host.dispatch(
        Lv2Request::ConfigUnregisterService { handle, service },
        src(),
        &rt,
    );
    assert_eq!(code_of(&again), u64::from(errno::CELL_ESRCH));
}

#[test]
fn removing_a_listener_drops_its_records_and_collects_the_dead_service() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let listener = written_u32(&add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    ));
    let service = written_u32(&register_user_service(&mut host, &rt, handle));
    host.dispatch(
        Lv2Request::ConfigUnregisterService { handle, service },
        src(),
        &rt,
    );
    let events = drain(&mut host, queue);
    assert_eq!(events.len(), 2);
    let d = host.dispatch(
        Lv2Request::ConfigRemoveServiceListener { handle, listener },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), 0);
    assert_eq!(host.state.config.event_count(), 0);
    assert!(host.state.config.service(service).is_none());
    for ev in events {
        let d = get_record(&mut host, &rt, handle, ev.data2 as u32, ev.data3);
        assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
    }
    let d = host.dispatch(
        Lv2Request::ConfigRemoveServiceListener { handle, listener },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
}

#[test]
fn close_stops_record_reads_but_not_delivery() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let d = host.dispatch(Lv2Request::ConfigClose { handle }, src(), &rt);
    assert_eq!(code_of(&d), 0);
    let d = host.dispatch(Lv2Request::ConfigClose { handle }, src(), &rt);
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
    let other = open(&mut host, &rt, queue);
    register_user_service(&mut host, &rt, other);
    let ev = drain(&mut host, queue);
    assert_eq!(ev.len(), 1, "the closed handle's listener still delivers");
    let d = get_record(&mut host, &rt, handle, ev[0].data2 as u32, ev[0].data3);
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
}

#[test]
fn listener_data_over_the_cap_is_einval_and_an_unreadable_buffer_is_efault() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 8);
    let handle = open(&mut host, &rt, queue);
    let d = add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER,
        PAD_FILTER_PTR,
        SYS_CONFIG_DATA_CAP + 1,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_EINVAL));
    assert_eq!(
        host.obs.invariant_break_sites["dispatch.config_data_over_cap"],
        1
    );
    let d = add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_PADMANAGER,
        0xffff_fff0,
        0x20,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_EFAULT));
    assert_eq!(host.state.config.event_count(), 0);
}

#[test]
fn a_full_queue_drops_the_event_and_counts_it() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let queue = create_queue(&mut host, &rt, 1);
    let handle = open(&mut host, &rt, queue);
    add_listener(
        &mut host,
        &rt,
        handle,
        SYS_CONFIG_SERVICE_USER_LIBPAD,
        0,
        0,
        SYS_CONFIG_SERVICE_LISTENER_REPEATING,
    );
    let first = register_user_service(&mut host, &rt, handle);
    assert_eq!(code_of(&first), 0);
    let second = register_user_service(&mut host, &rt, handle);
    assert_eq!(code_of(&second), 0, "the register itself succeeds");
    assert_eq!(host.obs.config_events_dropped, 1);
    assert_eq!(host.state.config.event_count(), 1);
    assert_eq!(drain(&mut host, queue).len(), 1);
}

#[test]
fn config_state_folds_into_the_host_hash_only_once_touched() {
    let mut host = Lv2Host::new();
    let rt = runtime();
    let pristine = host.state_hash();
    let queue = create_queue(&mut host, &rt, 8);
    let after_queue = host.state_hash();
    assert_ne!(pristine, after_queue);
    let mut twin = host.clone();
    open(&mut host, &rt, queue);
    assert_ne!(host.state_hash(), after_queue);
    open(&mut twin, &rt, queue);
    assert_eq!(
        host.state_hash(),
        twin.state_hash(),
        "same steps, same hash"
    );
}
