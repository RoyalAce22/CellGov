//! A spawned child's exit hands back the kernel ids it held: the
//! parent's next creates take the child's slots, one generation up.

use cellgov_event::UnitId;
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::kernel_id::{compose, generation_of, slot_of};
use cellgov_ps3_abi::lv2::memory::{page_size::FLAG_64K, SYS_MMAPPER_NO_SHM_KEY};
use cellgov_ps3_abi::lv2::process::SYS_TIMER_OBJECT;
use cellgov_ps3_abi::lv2::syscall;

use super::super::spaces::AddressSpaceId;
use super::super::Runtime;
use super::tests::{build_spawn_ready, spawn_request};

const OUT: u64 = 0x400;

/// The u32 a call wrote at `OUT` in `space`.
fn read_word(rt: &Runtime, space: AddressSpaceId) -> u32 {
    let bytes = rt
        .space_memory(space)
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(OUT), 4).unwrap())
        .expect("the call wrote its word");
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: [u64; 4]) -> Option<u64> {
    let [a, b, c, d] = args;
    rt.dispatch_lv2_request(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit);
    rt.registry_mut().drain_syscall_return(unit)
}

/// Create through `number` from `unit` and read the id it wrote into
/// `space`.
fn create(rt: &mut Runtime, unit: UnitId, space: AddressSpaceId, number: u64) -> u32 {
    assert_eq!(call(rt, unit, number, [OUT, 0, 0, 0]), Some(0), "{number}");
    let bytes = rt
        .space_memory(space)
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(OUT), 4).unwrap())
        .expect("the create wrote its id");
    u32::from_be_bytes(bytes.try_into().unwrap())
}

#[test]
fn a_spawned_childs_exit_hands_its_ids_back_to_the_parents_creates() {
    let mut rt = build_spawn_ready();
    let parent = UnitId::new(0);
    rt.dispatch_lv2_request(spawn_request(), parent);
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    let held = [
        create(&mut rt, child, child_space, syscall::TIMER_CREATE),
        create(&mut rt, child, child_space, syscall::RWLOCK_CREATE),
    ];

    rt.dispatch_lv2_request(
        classify(syscall::PROCESS_EXIT, &[0, 0, 0, 0, 0, 0, 0, 0]),
        child,
    );

    for (n, freed) in held.into_iter().enumerate() {
        assert_eq!(
            create(&mut rt, parent, AddressSpaceId::BOOT, syscall::TIMER_CREATE),
            compose(SYS_TIMER_OBJECT, generation_of(freed) + 1, slot_of(freed)),
            "parent create {n} takes the child's slot one generation up",
        );
    }
    let host = rt.lv2_host();
    assert_eq!(host.observability().process_exit_retained_ids, 0);
    assert_eq!(host.observability().invariant_break_count, 0);
}

#[test]
fn a_childs_exit_frees_its_keyless_mapped_segment_without_parking_it() {
    let mut rt = build_spawn_ready();
    let parent = UnitId::new(0);
    rt.dispatch_lv2_request(spawn_request(), parent);
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    let area = [0x1000_0000, FLAG_64K, 0x1000_0000, OUT];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_ALLOCATE_ADDRESS, area),
        Some(0)
    );
    let base = u64::from(read_word(&rt, child_space));
    let shm = [SYS_MMAPPER_NO_SHM_KEY, 0x1_0000, FLAG_64K, OUT];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_ALLOCATE_SHARED_MEMORY, shm),
        Some(0)
    );
    let mem_id = u64::from(read_word(&rt, child_space));
    let map = [base, mem_id, 0x40000, 0];
    assert_eq!(
        call(&mut rt, child, syscall::MMAPPER_MAP_SHARED_MEMORY, map),
        Some(0)
    );

    rt.dispatch_lv2_request(
        classify(syscall::PROCESS_EXIT, &[0, 0, 0, 0, 0, 0, 0, 0]),
        child,
    );

    let free = [mem_id, 0, 0, 0];
    assert_eq!(
        call(&mut rt, parent, syscall::MMAPPER_FREE_SHARED_MEMORY, free),
        Some(u64::from(cellgov_ps3_abi::lv2::errno::CELL_ESRCH)),
        "the exit freed the segment",
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}
