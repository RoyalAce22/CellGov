//! A child process's exit takes its shared-memory windows out of its
//! space, and each segment keeps the bytes its window held.

use cellgov_event::UnitId;
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::memory::{page_size::FLAG_64K, SYS_MMAPPER_NO_SHM_KEY};
use cellgov_ps3_abi::lv2::syscall;

use super::super::spaces::AddressSpaceId;
use super::super::Runtime;
use super::tests::{build_spawn_ready, spawn_request};

const AREA: u64 = 0x1000_0000;
const PROT_RW: u64 = 0x40000;
const OUT: u64 = 0x400;
const ID_OUT: u64 = 0x404;
const MARK: u32 = 0x1234_5678;

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: [u64; 4]) -> Option<u64> {
    let [a, b, c, d] = args;
    rt.dispatch_lv2_request(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit);
    rt.registry_mut().drain_syscall_return(unit)
}

fn word(rt: &Runtime, space: AddressSpaceId, addr: u64) -> Option<u32> {
    let bytes = rt
        .space_memory(space)
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())?;
    Some(u32::from_be_bytes(bytes.try_into().unwrap()))
}

#[test]
fn a_childs_exit_removes_its_windows_and_its_segment_keeps_their_bytes() {
    let mut rt = build_spawn_ready();
    let parent = UnitId::new(0);
    rt.dispatch_lv2_request(spawn_request(), parent);
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    let reserve = [AREA, FLAG_64K, AREA, OUT];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_ALLOCATE_ADDRESS, reserve),
        Some(0)
    );
    let a = u64::from(word(&rt, child_space, OUT).expect("330 wrote the base"));
    let shm = [SYS_MMAPPER_NO_SHM_KEY, 0x1_0000, FLAG_64K, ID_OUT];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_ALLOCATE_SHARED_MEMORY, shm),
        Some(0)
    );
    let m = u64::from(word(&rt, child_space, ID_OUT).expect("332 wrote the id"));
    let map = [a, m, PROT_RW, 0];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_MAP_SHARED_MEMORY, map),
        Some(0)
    );
    rt.space_memory_mut(child_space)
        .unwrap()
        .apply_commit(
            ByteRange::new(GuestAddr::new(a + 0x10), 4).unwrap(),
            &MARK.to_be_bytes(),
        )
        .expect("the window is mapped");

    rt.dispatch_lv2_request(
        classify(syscall::PROCESS_EXIT, &[0, 0, 0, 0, 0, 0, 0, 0]),
        child,
    );
    assert_eq!(
        word(&rt, child_space, a + 0x10),
        None,
        "the child's window is gone"
    );

    assert_eq!(
        call(&mut rt, parent, syscall::MMAPPER_ALLOCATE_ADDRESS, reserve),
        Some(0)
    );
    let pa = u64::from(word(&rt, AddressSpaceId::BOOT, OUT).expect("330 wrote the base"));
    assert_eq!(
        call(
            &mut rt,
            parent,
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            [pa, m, PROT_RW, 0]
        ),
        Some(0)
    );
    assert_eq!(
        word(&rt, AddressSpaceId::BOOT, pa + 0x10),
        Some(MARK),
        "the segment kept what the child wrote"
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}
