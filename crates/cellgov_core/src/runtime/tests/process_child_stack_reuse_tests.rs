//! A spawned process reuses a collected thread's stack range: the
//! new thread's block replaces the old thread's stack region in the
//! process's space.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr, PageSize};
use cellgov_ps3_abi::lv2::errno::CELL_ENOMEM;
use cellgov_ps3_abi::lv2::ppu_thread::SYS_PPU_THREAD_CREATE_JOINABLE;
use cellgov_ps3_abi::lv2::syscall;

use super::super::spaces::AddressSpaceId;
use super::super::Runtime;
use super::tests::{build_spawn_ready, spawn_request};

const TID_OUT: u64 = 0x30;
const STATUS_OUT: u64 = 0x600;
const CHILD: UnitId = UnitId::new(1);
const CHILD_SPACE: AddressSpaceId = AddressSpaceId::new(1);

/// A spawned child with a thread param block and its OPD staged in the
/// child's space.
fn spawned() -> Runtime {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let mem = rt.space_memory_mut(CHILD_SPACE).unwrap();
    let mut write = |addr: u64, bytes: &[u8]| {
        let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
        mem.apply_commit(range, bytes).unwrap();
    };
    write(0x200, &0x210u32.to_be_bytes()); // param.entry_opd_ptr
    write(0x204, &0u32.to_be_bytes()); // param.tls
    write(0x210, &0x300u32.to_be_bytes()); // OPD code
    write(0x214, &0x400u32.to_be_bytes()); // OPD toc
    rt
}

/// Create a joinable thread with a `stack`-byte stack from the child;
/// returns its guest id and stack base.
fn create(rt: &mut Runtime, unit: UnitId, stack: u64) -> (u64, u64) {
    let req = classify(
        syscall::PPU_THREAD_CREATE,
        &[
            TID_OUT,
            0x200,
            0,
            0,
            1000,
            stack,
            SYS_PPU_THREAD_CREATE_JOINABLE,
            0,
        ],
    );
    rt.dispatch_lv2_request(req, CHILD);
    assert_eq!(rt.registry_mut().drain_syscall_return(CHILD), Some(0));
    let bytes = rt
        .space_memory(CHILD_SPACE)
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(TID_OUT), 8).unwrap())
        .unwrap()
        .to_vec();
    let tid = u64::from_be_bytes(bytes.try_into().unwrap());
    let base = rt
        .lv2_host()
        .ppu_thread_for_unit(unit)
        .expect("the created thread has a record")
        .attrs
        .stack_base;
    (tid, u64::from(base))
}

/// The child joins thread `tid`, run by `unit`, which exits.
fn join(rt: &mut Runtime, tid: u64, unit: UnitId) {
    let join = classify(
        syscall::PPU_THREAD_JOIN,
        &[tid, STATUS_OUT, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(join, CHILD);
    assert_eq!(
        rt.registry().effective_status(CHILD),
        Some(UnitStatus::Blocked)
    );
    let exit = classify(syscall::PPU_THREAD_EXIT, &[0, 0, 0, 0, 0, 0, 0, 0]);
    rt.dispatch_lv2_request(exit, unit);
    assert_eq!(rt.registry_mut().drain_syscall_return(CHILD), Some(0));
}

#[test]
fn a_bigger_stack_at_a_joined_threads_address_replaces_its_region() {
    let mut rt = spawned();
    let first = UnitId::new(2);
    let (tid, base) = create(&mut rt, first, 0x4000);
    join(&mut rt, tid, first);

    // The joined thread's range is the lowest free one, and the new
    // block is bigger than the old region there.
    let (_, again) = create(&mut rt, UnitId::new(3), 0x8000);
    assert_eq!(again, base);
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.ppu_thread_create_stack_install_overlap"),
        0,
        "the old stack region must not refuse the new one",
    );
    assert!(
        rt.space_memory(CHILD_SPACE)
            .unwrap()
            .read(ByteRange::new(GuestAddr::new(base), 0x8000).unwrap())
            .is_some(),
        "the whole new block is one readable range in the child's space",
    );
}

#[test]
fn a_create_refused_at_stack_install_frees_its_block() {
    let mut rt = spawned();
    let first = UnitId::new(2);
    let (tid, base) = create(&mut rt, first, 0x4000);
    join(&mut rt, tid, first);

    // Put a region that is not a thread stack where the next block goes.
    let mem = rt.space_memory_mut(CHILD_SPACE).unwrap();
    mem.remove_region(base, 0x4000).unwrap();
    mem.install_region(base, 0x1000, "blocker", PageSize::Page4K)
        .unwrap();
    let req = classify(
        syscall::PPU_THREAD_CREATE,
        &[
            TID_OUT,
            0x200,
            0,
            0,
            1000,
            0x4000,
            SYS_PPU_THREAD_CREATE_JOINABLE,
            0,
        ],
    );
    rt.dispatch_lv2_request(req, CHILD);
    assert_eq!(
        rt.registry_mut().drain_syscall_return(CHILD),
        Some(CELL_ENOMEM.into())
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.ppu_thread_create_stack_install_overlap"),
        1,
    );
    assert!(
        rt.lv2_host().ppu_thread_for_unit(UnitId::new(3)).is_none(),
        "the refused thread leaves the table",
    );

    // The refused thread holds no block, so the next create gets it.
    rt.space_memory_mut(CHILD_SPACE)
        .unwrap()
        .remove_region(base, 0x1000)
        .unwrap();
    let (_, again) = create(&mut rt, UnitId::new(4), 0x4000);
    assert_eq!(again, base);
}

#[test]
fn a_live_threads_stack_region_is_left_in_place() {
    let mut rt = spawned();
    let (_, low) = create(&mut rt, UnitId::new(2), 0x4000);
    let (_, high) = create(&mut rt, UnitId::new(3), 0x4000);
    assert_eq!(high, low + 0x4000);
    let mem = rt.space_memory(CHILD_SPACE).unwrap();
    for base in [low, high] {
        assert!(
            mem.read(ByteRange::new(GuestAddr::new(base), 0x4000).unwrap())
                .is_some(),
            "the stack at {base:#x} stays mapped",
        );
    }
}
