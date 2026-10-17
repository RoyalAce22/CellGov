//! Mmapper areas and windows belong to the process that made them: a
//! child reserves its own areas from the region start, cannot map into
//! or unmap from its parent's, and its exit releases what it held.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
use cellgov_ps3_abi::lv2::{errno, syscall};

use crate::host::process::ProcessEntry;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const CHILD: u32 = BOOT_PROCESS_PID + 0x100;
const PARENT_UNIT: UnitId = UnitId::new(0);
const CHILD_UNIT: UnitId = UnitId::new(1);
const KEYLESS: u64 = 0xFFFF_0000_0000_0000;
const FLAG_64K: u64 = 0x200;
const PROT_RW: u64 = 0x40000;
const AREA: u64 = 0x1000_0000;
const OUT: u64 = 0x100;

/// A host with a child process bound to `CHILD_UNIT`.
fn host_with_child() -> Lv2Host {
    let mut host = Lv2Host::new();
    assert!(host.state.processes.insert_child(
        CHILD,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
        },
    ));
    host.bind_unit_process(CHILD_UNIT, CHILD);
    host
}

/// One call from `unit`: the return code and the u32 of its first write.
fn sc(host: &mut Lv2Host, unit: UnitId, number: u64, args: [u64; 4]) -> (u64, Option<u64>) {
    let rt = FakeRuntime::new(0x1000);
    let [a, b, c, d] = args;
    match host.dispatch(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit, &rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects.iter().find_map(|e| match e {
                Effect::SharedWriteIntent { bytes, .. } => Some(u64::from(u32::from_be_bytes(
                    bytes.bytes()[..4].try_into().expect("a u32 write"),
                ))),
                _ => None,
            });
            (code, written)
        }
        other => panic!("syscall {number}: expected Immediate, got {other:?}"),
    }
}

fn reserve(host: &mut Lv2Host, unit: UnitId) -> u64 {
    let (code, base) = sc(
        host,
        unit,
        syscall::MMAPPER_ALLOCATE_ADDRESS,
        [AREA, FLAG_64K, AREA, OUT],
    );
    assert_eq!(code, 0, "330");
    base.expect("330 writes the base")
}

fn segment(host: &mut Lv2Host, unit: UnitId) -> u64 {
    let (code, id) = sc(
        host,
        unit,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [KEYLESS, 0x1_0000, FLAG_64K, OUT],
    );
    assert_eq!(code, 0, "332");
    id.expect("332 writes the id")
}

fn rc(host: &mut Lv2Host, unit: UnitId, number: u64, args: [u64; 4]) -> u64 {
    sc(host, unit, number, args).0
}

#[test]
fn a_child_reserves_its_first_area_at_the_base_a_game_process_gets() {
    let console = console_payload("lv2_mmapper_map");
    let console_base = u64::from_be_bytes(console[8..16].try_into().expect("slot 1"));
    let mut host = host_with_child();
    assert_eq!(reserve(&mut host, PARENT_UNIT), console_base);
    // The next granule holds the RSX context window.
    assert_eq!(reserve(&mut host, PARENT_UNIT), console_base + 2 * AREA);
    assert_eq!(
        reserve(&mut host, CHILD_UNIT),
        console_base,
        "the child's own first area"
    );
}

#[test]
fn a_child_maps_only_into_its_own_areas() {
    let mut host = host_with_child();
    let a = reserve(&mut host, PARENT_UNIT);
    let m = segment(&mut host, PARENT_UNIT);
    let einval = u64::from(errno::CELL_EINVAL);
    let map = [a, m, PROT_RW, 0];
    assert_eq!(
        rc(
            &mut host,
            CHILD_UNIT,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            map
        ),
        einval,
        "the parent's area is no area of the child's"
    );
    assert_eq!(
        reserve(&mut host, CHILD_UNIT),
        a,
        "the child's area at the same base"
    );
    assert_eq!(
        rc(
            &mut host,
            CHILD_UNIT,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            map
        ),
        0
    );
    assert_eq!(
        rc(
            &mut host,
            PARENT_UNIT,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            map
        ),
        0,
        "the same address in the parent is free"
    );
    assert_eq!(
        rc(
            &mut host,
            CHILD_UNIT,
            syscall::MMAPPER_FREE_ADDRESS,
            [a, 0, 0, 0]
        ),
        u64::from(errno::CELL_EBUSY),
        "the child's window holds the child's area"
    );
}

#[test]
fn a_child_unmaps_only_its_own_windows() {
    let mut host = host_with_child();
    let a = reserve(&mut host, PARENT_UNIT);
    let m = segment(&mut host, PARENT_UNIT);
    assert_eq!(
        rc(
            &mut host,
            PARENT_UNIT,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            [a, m, PROT_RW, 0]
        ),
        0
    );
    let unmap = [a, OUT, 0, 0];
    assert_eq!(
        rc(
            &mut host,
            CHILD_UNIT,
            syscall::MMAPPER_UNMAP_SHARED_MEMORY,
            unmap
        ),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        sc(
            &mut host,
            PARENT_UNIT,
            syscall::MMAPPER_UNMAP_SHARED_MEMORY,
            unmap
        ),
        (0, Some(m))
    );
}

#[test]
fn a_childs_exit_releases_its_windows_and_areas() {
    let mut host = host_with_child();
    let m = segment(&mut host, PARENT_UNIT);
    let a = reserve(&mut host, CHILD_UNIT);
    assert_eq!(
        rc(
            &mut host,
            CHILD_UNIT,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            [a, m, PROT_RW, 0]
        ),
        0
    );
    let free = [m, 0, 0, 0];
    assert_eq!(
        rc(
            &mut host,
            PARENT_UNIT,
            syscall::MMAPPER_FREE_SHARED_MEMORY,
            free
        ),
        u64::from(errno::CELL_EBUSY),
        "the child's window holds the segment"
    );

    host.mark_process_exited(CHILD, 0);

    assert_eq!(host.mmapper_areas_of(CHILD).count(), 0);
    assert_eq!(host.mmapper_windows_of(CHILD).count(), 0);
    assert_eq!(
        rc(
            &mut host,
            PARENT_UNIT,
            syscall::MMAPPER_FREE_SHARED_MEMORY,
            free
        ),
        0
    );
}

#[test]
fn a_childs_exit_releases_its_1m_page_area() {
    let mut host = host_with_child();
    let (code, block) = sc(
        &mut host,
        CHILD_UNIT,
        syscall::MEMORY_ALLOCATE,
        [
            0x10_0000,
            cellgov_ps3_abi::lv2::memory::page_size::FLAG_1M,
            OUT,
            0,
        ],
    );
    assert_eq!(code, 0, "348");
    let start = u64::from(Lv2Host::MMAPPER_REGION_START);
    assert_eq!(
        block,
        Some(start),
        "the child's area takes the lowest granule"
    );

    host.mark_process_exited(CHILD, 0);

    assert_eq!(
        reserve(&mut host, PARENT_UNIT),
        start,
        "the exited child's area no longer holds the granule"
    );
}
