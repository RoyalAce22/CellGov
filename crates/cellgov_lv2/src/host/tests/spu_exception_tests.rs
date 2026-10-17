//! The exception event an SPU thread's MFC exception sends its group's
//! exception queue: the words the `spu_list_bad_size` and `spu_ea_bound`
//! captures read, and no event for a group with no exception queue.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::event;

use super::*;
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;
use crate::sync_primitives::EventQueueReceive;
use crate::thread_group::MAX_SLOTS_PER_GROUP;

const GROUP: u32 = 1;
const UNIT: UnitId = UnitId::new(7);
const NPC: u32 = 0x238;

/// Group 1 with one thread, started, its SPU registered as [`UNIT`].
fn running_group() -> (Lv2Host, FakeRuntime) {
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
    (host, rt)
}

fn queue(host: &mut Lv2Host, rt: &FakeRuntime) -> u32 {
    match host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x200,
            attr_ptr: 0,
            key: 0,
            size: 8,
        },
        UnitId::new(0),
        rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn an_exception_sends_the_connected_queue_the_group_thread_npc_cause_and_address() {
    let (mut host, rt) = running_group();
    let queue = queue(&mut host, &rt);
    let connected = host.dispatch(
        Lv2Request::SpuThreadGroupConnectEvent {
            group_id: GROUP,
            queue_id: queue,
            event_type: event::GROUP_EXCEPTION,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(connected, Lv2Dispatch::Immediate { code: 0, .. }));

    let sent = host
        .raise_spu_group_exception(UNIT, NPC, event::EXCEPTION_MFC_SEGMENT, 0x2000_0000)
        .expect("the group has an exception queue");
    assert_eq!(sent.wake, None, "no receiver waits on the queue");
    let Some(EventQueueReceive::Delivered(ev)) = host.state.event_queues.try_receive(queue) else {
        panic!("the queue holds the event");
    };
    let thread = MAX_SLOTS_PER_GROUP * GROUP;
    assert_eq!(ev.source, event::GROUP_EXCEPTION_KEY);
    assert_eq!(ev.data1, u64::from(GROUP) << 32 | u64::from(thread));
    assert_eq!(
        ev.data2,
        u64::from(NPC) << 32 | u64::from(event::EXCEPTION_MFC_SEGMENT)
    );
    assert_eq!(ev.data3, 0x2000_0000);
}

#[test]
fn a_group_with_no_exception_queue_gets_no_event() {
    let (mut host, rt) = running_group();
    let queue = queue(&mut host, &rt);
    let connected = host.dispatch(
        Lv2Request::SpuThreadGroupConnectEvent {
            group_id: GROUP,
            queue_id: queue,
            event_type: event::GROUP_RUN,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(connected, Lv2Dispatch::Immediate { code: 0, .. }));
    assert_eq!(
        host.raise_spu_group_exception(UNIT, NPC, event::EXCEPTION_DMA_ALIGNMENT, 0),
        None
    );
    assert!(!matches!(
        host.state.event_queues.try_receive(queue),
        Some(EventQueueReceive::Delivered(_))
    ));
}
