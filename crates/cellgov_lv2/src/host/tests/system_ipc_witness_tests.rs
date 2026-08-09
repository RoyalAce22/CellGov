//! Non-vacuity for the two-channel system-IPC namespace witnesses:
//! each counter increments on a namespace key and stays zero on a
//! neighbouring key one namespace over.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::cell_errors;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    connected_port, create_mutex_host, extract_write_u32, fake_runtime_with_valid_sync_attr,
    seed_primary_ppu, FakeRuntime, VALID_SYNC_ATTR_PTR,
};
use crate::host::{is_system_ipc_key, Lv2Host};
use crate::request::Lv2Request;

/// A key inside the namespace, and one in the neighbouring
/// `0x8006_0200` namespace the RPCS3 vsh capture also saw traffic on.
const NS_KEY: u64 = 0x8006_0100_0000_0010;
const OTHER_KEY: u64 = 0x8006_0200_0000_0010;
const NS_COND_KEY: u64 = 0x8006_0100_0000_0030;
const OTHER_COND_KEY: u64 = 0x8006_0200_0000_0030;
const SHM_SIZE: u64 = 0x1_0000;

#[test]
fn the_namespace_predicate_admits_only_the_system_ipc_prefix() {
    assert!(is_system_ipc_key(NS_KEY));
    assert!(is_system_ipc_key(0x8006_0100_0000_ffff));
    assert!(!is_system_ipc_key(OTHER_KEY));
    assert!(!is_system_ipc_key(0x8006_0100_0001_0000));
    assert!(!is_system_ipc_key(0));
}

fn alloc_shm(host: &mut Lv2Host, rt: &FakeRuntime, ipc_key: u64) -> u32 {
    let d = host.dispatch(
        Lv2Request::Unsupported {
            number: 332,
            args: [ipc_key, SHM_SIZE, 0x200, 0x900, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    );
    match &d {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

fn map_shm(host: &mut Lv2Host, rt: &FakeRuntime, mem_id: u32, addr: u64) {
    let d = host.dispatch(
        Lv2Request::Unsupported {
            number: 334,
            args: [addr, u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    );
    assert!(
        matches!(d, Lv2Dispatch::Immediate { code: 0, .. }),
        "map failed: {d:?}"
    );
}

#[test]
fn a_keyed_shm_create_and_map_bump_the_channel_one_witnesses() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let mem_id = alloc_shm(&mut host, &rt, NS_KEY);
    map_shm(&mut host, &rt, mem_id, 0x3000_0000);

    let w = host.system_ipc_witness();
    assert_eq!((w.shm_creates, w.shm_attaches, w.shm_maps), (1, 0, 1));
    assert_eq!(w.keys_touched.get(&NS_KEY), Some(&2));
}

#[test]
fn a_second_keyed_create_counts_as_an_attach_not_a_create() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let first = alloc_shm(&mut host, &rt, NS_KEY);
    let second = alloc_shm(&mut host, &rt, NS_KEY);
    assert_eq!(first, second, "a registered key resolves to the same shm");

    let w = host.system_ipc_witness();
    assert_eq!((w.shm_creates, w.shm_attaches), (1, 1));
}

#[test]
fn a_shm_outside_the_namespace_leaves_every_witness_silent() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let mem_id = alloc_shm(&mut host, &rt, OTHER_KEY);
    map_shm(&mut host, &rt, mem_id, 0x3000_0000);
    assert!(host.system_ipc_witness().is_silent());
}

fn write_effect(addr: u32, len: usize) -> Effect {
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(u64::from(addr)), len as u64).unwrap(),
        bytes: WritePayload::from_slice(&vec![0u8; len]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::ZERO,
    }
}

#[test]
fn a_committed_write_inside_the_mapping_bumps_the_shm_write_witness() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let mem_id = alloc_shm(&mut host, &rt, NS_KEY);
    map_shm(&mut host, &rt, mem_id, 0x3000_0000);

    // Inside, straddling the low edge, and the last byte of the range.
    host.note_committed_effects(&[
        write_effect(0x3000_0040, 4),
        write_effect(0x2fff_fffc, 8),
        write_effect(0x3000_ffff, 1),
    ]);
    assert_eq!(host.system_ipc_witness().shm_writes, 3);
}

#[test]
fn a_committed_write_outside_the_mapping_does_not_bump_the_shm_write_witness() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let mem_id = alloc_shm(&mut host, &rt, NS_KEY);
    map_shm(&mut host, &rt, mem_id, 0x3000_0000);

    // One byte short of the base, and one byte past the end.
    host.note_committed_effects(&[write_effect(0x2fff_fffc, 4), write_effect(0x3001_0000, 4)]);
    assert_eq!(host.system_ipc_witness().shm_writes, 0);
}

#[test]
fn the_shm_write_witness_is_blind_to_writes_through_an_unrecorded_alias() {
    // The mapping ledger is fed by 334 / 337 only. A write reaching
    // the same shm through any other route tests against no range and
    // goes uncounted -- the documented limit of an address-based test.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let mem_id = alloc_shm(&mut host, &rt, NS_KEY);
    map_shm(&mut host, &rt, mem_id, 0x3000_0000);

    let alias_base = 0x4000_0000u32;
    host.note_committed_effects(&[write_effect(alias_base, 4)]);
    assert_eq!(host.system_ipc_witness().shm_writes, 0);

    // Same bytes, same shm, counted once the alias is a recorded map.
    map_shm(&mut host, &rt, mem_id, u64::from(alias_base));
    host.note_committed_effects(&[write_effect(alias_base, 4)]);
    assert_eq!(host.system_ipc_witness().shm_writes, 1);
}

/// `FakeRuntime` carrying a process-shared `sys_cond_attribute_t` at
/// 0x800 whose ipc_key is `ipc_key`.
fn cond_attr_runtime(ipc_key: u64) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    let commit = |mem: &mut GuestMemory, addr: u64, bytes: &[u8]| {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    };
    commit(&mut mem, 0x800, &0x100u32.to_be_bytes());
    commit(&mut mem, 0x808, &ipc_key.to_be_bytes());
    FakeRuntime::with_memory(mem)
}

/// Create a keyed cond and signal it; returns the host so the caller
/// can read the witness. The signal lands on an empty waiter list,
/// which is the traffic shape the drain-time signals take.
fn keyed_cond_create_and_signal(ipc_key: u64) -> Lv2Host {
    let mut host = Lv2Host::new();
    let rt = cond_attr_runtime(ipc_key);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0x800,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::CondSignal { id: cond_id }, src, &rt);
    host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, src, &rt);
    host
}

#[test]
fn a_namespace_cond_create_and_signal_bump_the_cond_witnesses() {
    let host = keyed_cond_create_and_signal(NS_COND_KEY);
    let w = host.system_ipc_witness();
    assert_eq!((w.cond_creates, w.cond_signals), (1, 2));
    assert_eq!(w.cond_waits, 0);
}

#[test]
fn a_cond_outside_the_namespace_leaves_the_cond_witnesses_silent() {
    let host = keyed_cond_create_and_signal(OTHER_COND_KEY);
    assert!(host.system_ipc_witness().is_silent());
}

#[test]
fn a_namespace_cond_wait_bumps_the_wait_witness() {
    let mut host = Lv2Host::new();
    let rt = cond_attr_runtime(NS_COND_KEY);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0x800,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert_eq!(host.system_ipc_witness().cond_waits, 1);
}

/// Create an IPC-type port (the type `sys_event_port_connect_ipc`
/// requires).
fn ipc_port(host: &mut Lv2Host, rt: &FakeRuntime, src: UnitId) -> u32 {
    let created = host.dispatch(
        Lv2Request::EventPortCreate {
            id_ptr: 0x380,
            port_type: crate::sync_primitives::SYS_EVENT_PORT_IPC as u32,
            name: 0,
        },
        src,
        rt,
    );
    match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

fn create_event_queue(host: &mut Lv2Host, rt: &FakeRuntime, key: u64) -> u32 {
    let created = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x300,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            key,
            size: 4,
        },
        UnitId::new(0),
        rt,
    );
    match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn a_namespace_event_queue_create_and_enqueue_bump_channel_two() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let queue_id = create_event_queue(&mut host, &rt, NS_KEY);
    let port_id = connected_port(&mut host, &rt, src, queue_id);
    let sent = host.dispatch(
        Lv2Request::EventPortSend {
            port_id,
            data1: 1,
            data2: 2,
            data3: 3,
        },
        src,
        &rt,
    );
    assert!(matches!(sent, Lv2Dispatch::Immediate { code: 0, .. }));

    let w = host.system_ipc_witness();
    assert_eq!(
        (w.event_queue_creates, w.event_queue_enqueues),
        (1, 1),
        "a keyed create and a delivery through a connected port"
    );
}

/// The oracle passes `SYS_SYNC_NEWLY_CREATED` unconditionally
/// (RPCS3 `sys_event.cpp:251`), so a duplicate key is refused rather
/// than resolved. The witness still records the attempt.
#[test]
fn a_second_keyed_create_on_the_same_key_is_eexist() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    create_event_queue(&mut host, &rt, NS_KEY);
    let second = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x300,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            key: NS_KEY,
            size: 4,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        second,
        Lv2Dispatch::immediate(cell_errors::CELL_EEXIST.into())
    );
    let w = host.system_ipc_witness();
    assert_eq!((w.event_queue_creates, w.event_queue_references), (1, 1));
}

/// Connecting by key is how a second referent reaches the queue --
/// the path a duplicate create does NOT provide.
#[test]
fn connect_ipc_resolves_a_namespace_key_and_bumps_the_connect_witness() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let queue_id = create_event_queue(&mut host, &rt, NS_KEY);
    let port_id = ipc_port(&mut host, &rt, src);

    let connected = host.dispatch(
        Lv2Request::Unsupported {
            number: 140,
            args: [u64::from(port_id), NS_KEY, 0, 0, 0, 0, 0, 0],
        },
        src,
        &rt,
    );
    assert_eq!(connected, Lv2Dispatch::immediate(0));
    assert_eq!(
        host.event_ports.lookup(port_id).unwrap().queue(),
        Some(queue_id)
    );
    assert_eq!(host.system_ipc_witness().event_port_connects, 1);
}

#[test]
fn connect_ipc_to_an_unregistered_namespace_key_is_esrch_and_still_witnessed() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let port_id = ipc_port(&mut host, &rt, src);
    let connected = host.dispatch(
        Lv2Request::Unsupported {
            number: 140,
            args: [u64::from(port_id), NS_KEY, 0, 0, 0, 0, 0, 0],
        },
        src,
        &rt,
    );
    assert_eq!(
        connected,
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
    // Counted before the resolve, so a miss is visible -- this is the
    // reading a vsh boot produces today.
    assert_eq!(host.system_ipc_witness().event_port_connects, 1);
}

#[test]
fn an_event_queue_outside_the_namespace_leaves_channel_two_silent() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let queue_id = create_event_queue(&mut host, &rt, OTHER_KEY);
    let port_id = connected_port(&mut host, &rt, src, queue_id);
    host.dispatch(
        Lv2Request::EventPortSend {
            port_id,
            data1: 0,
            data2: 0,
            data3: 0,
        },
        src,
        &rt,
    );
    assert!(host.system_ipc_witness().is_silent());
}

#[test]
fn a_refused_send_is_not_counted_as_an_enqueue() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let queue_id = create_event_queue(&mut host, &rt, NS_KEY);
    let port_id = connected_port(&mut host, &rt, src, queue_id);
    let send = |host: &mut Lv2Host| {
        host.dispatch(
            Lv2Request::EventPortSend {
                port_id,
                data1: 0,
                data2: 0,
                data3: 0,
            },
            src,
            &rt,
        )
    };
    // Depth 4 with no receiver: the fifth send is refused.
    for _ in 0..4 {
        send(&mut host);
    }
    assert_eq!(
        send(&mut host),
        Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
    );
    assert_eq!(host.system_ipc_witness().event_queue_enqueues, 4);
}

#[test]
fn a_send_through_an_unconnected_port_is_enotconn() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    create_event_queue(&mut host, &rt, NS_KEY);
    let port_id = ipc_port(&mut host, &rt, src);
    let sent = host.dispatch(
        Lv2Request::EventPortSend {
            port_id,
            data1: 0,
            data2: 0,
            data3: 0,
        },
        src,
        &rt,
    );
    assert_eq!(
        sent,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOTCONN.into())
    );
    assert_eq!(host.system_ipc_witness().event_queue_enqueues, 0);
}
