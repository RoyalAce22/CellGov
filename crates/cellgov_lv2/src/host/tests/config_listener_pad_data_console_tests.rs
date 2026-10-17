//! The pad-manager listener data buffers held to the reference-console
//! capture `lv2_config_listener_pad_data`: which lengths register, and
//! which lead bytes hear the attached pad's service. The test drives the
//! probe's calls through dispatch and compares the return codes, the ids
//! written and the events each listener receives with the capture.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_SERVICE_LISTENER_REPEATING, SYS_CONFIG_SERVICE_PADMANAGER,
    SYS_CONFIG_SERVICE_PADMANAGER2,
};
use cellgov_ps3_abi::lv2::kernel_id;

use super::*;
use crate::host::test_support::{console_payload, extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::sync_primitives::{EventPayload, EventQueueReceive};
use crate::Lv2Dispatch;

const OUT_PTR: u32 = 0x100;
/// One 32-byte data buffer per lead byte the probe sends: the lead,
/// then zeroes.
const DATA_BASE: u32 = 0x800;
const LEADS: [u8; 4] = [0x01, 0x02, 0x04, 0x00];
/// The probe's SKIP word.
const SKIP: u64 = 0xDEAD_D00D;

fn words() -> Vec<u64> {
    console_payload("lv2_config_listener_pad_data")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn data_ptr(lead: u8) -> u32 {
    let i = LEADS
        .iter()
        .position(|&l| l == lead)
        .expect("a probed lead");
    DATA_BASE + 0x20 * i as u32
}

fn runtime() -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    for lead in LEADS {
        let mut data = [0u8; 32];
        data[0] = lead;
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(data_ptr(lead))), 32).unwrap(),
            &data,
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

#[test]
fn pad_manager_listener_buffers_register_and_hear_as_on_the_console() {
    let w = words();
    let rt = runtime();
    let mut host = host_from(w[1]);
    let (rc, queue) = created(
        &mut host,
        &rt,
        Lv2Request::EventQueueCreate {
            id_ptr: OUT_PTR,
            attr_ptr: 0,
            key: 0,
            size: 16,
        },
    );
    assert_eq!(rc, w[0], "queue create");
    let queue = queue.expect("a queue id");
    assert_eq!(u64::from(queue), w[1], "queue");
    let (rc, handle) = created(
        &mut host,
        &rt,
        Lv2Request::ConfigOpen {
            equeue_id: queue,
            out_handle_ptr: OUT_PTR,
        },
    );
    assert_eq!(rc, w[2], "open");
    let handle = handle.expect("a handle");
    assert_eq!(u64::from(handle), w[3], "handle");

    let cases = [
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x01, 26),
        (SYS_CONFIG_SERVICE_PADMANAGER2, 0x01, 26),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x02, 26),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x04, 26),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x00, 26),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x01, 25),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x01, 27),
        (SYS_CONFIG_SERVICE_PADMANAGER, 0x01, 32),
    ];
    let mut registered = 0;
    let mut heard = 0;
    for (n, (service_id, lead, size)) in cases.into_iter().enumerate() {
        let slot = 4 + 6 * n;
        let (rc, listener) = created(
            &mut host,
            &rt,
            Lv2Request::ConfigAddServiceListener {
                handle,
                service_id,
                min_verbosity: 1,
                in_ptr: data_ptr(lead),
                size,
                listener_type: SYS_CONFIG_SERVICE_LISTENER_REPEATING,
                out_listener_ptr: OUT_PTR,
            },
        );
        assert_eq!(rc, w[slot], "case {n}: add");
        assert_eq!(
            listener.map_or(u64::from(u32::MAX), u64::from),
            w[slot + 1],
            "case {n}: listener id"
        );
        let events = drain(&mut host, queue);
        assert_eq!(events.len() as u64, w[slot + 2], "case {n}: events");
        if let Some(ev) = events.first() {
            assert_eq!(ev.data2 >> 32, w[slot + 3] >> 32, "case {n}: registered");
            assert_eq!(ev.data3, w[slot + 4], "case {n}: announced length");
            heard += 1;
        } else {
            assert_eq!((w[slot + 3], w[slot + 4]), (SKIP, SKIP), "case {n}");
        }
        match listener {
            Some(listener) => {
                let (rc, _) = dispatch(
                    &mut host,
                    &rt,
                    Lv2Request::ConfigRemoveServiceListener { handle, listener },
                );
                assert_eq!(rc, w[slot + 5], "case {n}: remove");
                registered += 1;
            }
            None => assert_eq!(w[slot + 5], SKIP, "case {n}: nothing to remove"),
        }
        drain(&mut host, queue);
    }
    assert!(
        registered > 0 && heard > 0 && registered < cases.len(),
        "the capture registers some cases, refuses others, and some hear"
    );
}
