//! The shm-write witness keeps each process's mappings apart: two
//! processes can map at one base, and only a process's own writes,
//! unmaps and exit touch its entries.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::ipc::SYSTEM_IPC_KEY_NAMESPACE;
use cellgov_ps3_abi::lv2::memory::page_size::FLAG_64K;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
use cellgov_ps3_abi::lv2::syscall;
use cellgov_time::GuestTicks;

use crate::host::process::ProcessEntry;
use crate::host::test_support::{extract_write_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const CHILD: u32 = BOOT_PROCESS_PID + 0x100;
const PARENT_UNIT: UnitId = UnitId::new(0);
const CHILD_UNIT: UnitId = UnitId::new(1);
const NS_KEY: u64 = SYSTEM_IPC_KEY_NAMESPACE | 0x10;
const AREA: u64 = 0x1000_0000;
const OUT: u64 = 0x100;

fn host_with_child() -> Lv2Host {
    let mut host = Lv2Host::new();
    assert!(host.state.processes.insert_child(
        CHILD,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
            module_registered: false,
        },
    ));
    host.bind_unit_process(CHILD_UNIT, CHILD);
    host
}

/// One call from `unit`; panics unless it returns 0, and gives back the
/// u32 of its first write, if any.
fn ok(host: &mut Lv2Host, unit: UnitId, number: u64, args: [u64; 4]) -> Option<u32> {
    let rt = FakeRuntime::new(0x1000);
    let [a, b, c, d] = args;
    match host.dispatch(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects.first().map(extract_write_u32),
        other => panic!("syscall {number} from {unit:?}: {other:?}"),
    }
}

/// Reserve an area, attach the namespace shm, and map it at the area's
/// base; the base.
fn map_namespace_shm(host: &mut Lv2Host, unit: UnitId) -> u32 {
    let base = ok(
        host,
        unit,
        syscall::MMAPPER_ALLOCATE_ADDRESS,
        [AREA, FLAG_64K, AREA, OUT],
    )
    .expect("330 writes the base");
    let mem_id = ok(
        host,
        unit,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [NS_KEY, 0x1_0000, FLAG_64K, OUT],
    )
    .expect("332 writes the id");
    ok(
        host,
        unit,
        syscall::MMAPPER_MAP_SHARED_MEMORY,
        [u64::from(base), u64::from(mem_id), 0, 0],
    );
    base
}

fn write_at(addr: u32) -> Effect {
    Effect::shared_write(
        ByteRange::new(GuestAddr::new(u64::from(addr)), 4).unwrap(),
        WritePayload::from_slice(&[0u8; 4]),
        PARENT_UNIT,
        GuestTicks::ZERO,
    )
}

fn shm_writes(host: &Lv2Host) -> u64 {
    host.observability().system_ipc_witness.shm_writes
}

#[test]
fn a_write_counts_only_against_the_writing_process_mappings() {
    let mut host = host_with_child();
    let base = map_namespace_shm(&mut host, PARENT_UNIT);

    host.note_committed_effects(CHILD, &[write_at(base)]);
    assert_eq!(shm_writes(&host), 0, "the child maps nothing at that base");

    host.note_committed_effects(BOOT_PROCESS_PID, &[write_at(base)]);
    assert_eq!(shm_writes(&host), 1);
}

#[test]
fn one_process_unmap_at_a_shared_base_leaves_the_other_mapping() {
    let mut host = host_with_child();
    let parent_base = map_namespace_shm(&mut host, PARENT_UNIT);
    let child_base = map_namespace_shm(&mut host, CHILD_UNIT);
    assert_eq!(parent_base, child_base, "both areas start the region");

    ok(
        &mut host,
        CHILD_UNIT,
        syscall::MMAPPER_UNMAP_SHARED_MEMORY,
        [u64::from(child_base), OUT, 0, 0],
    );
    host.note_committed_effects(CHILD, &[write_at(child_base)]);
    assert_eq!(shm_writes(&host), 0, "the child's mapping is gone");
    host.note_committed_effects(BOOT_PROCESS_PID, &[write_at(parent_base)]);
    assert_eq!(shm_writes(&host), 1, "the parent's mapping stays");
}

#[test]
fn a_childs_exit_drops_its_mappings_and_keeps_the_parents() {
    let mut host = host_with_child();
    let parent_base = map_namespace_shm(&mut host, PARENT_UNIT);
    let child_base = map_namespace_shm(&mut host, CHILD_UNIT);

    host.mark_process_exited(CHILD, 0);

    host.note_committed_effects(CHILD, &[write_at(child_base)]);
    assert_eq!(shm_writes(&host), 0, "the exited child's mapping is gone");
    host.note_committed_effects(BOOT_PROCESS_PID, &[write_at(parent_base)]);
    assert_eq!(shm_writes(&host), 1);
}
