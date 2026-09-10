//! One rule binds the u32 guest fields of every `Unsupported` arm.

use super::*;
use cellgov_ps3_abi::lv2::syscall;

/// A register whose low word names a plausible guest address and whose
/// high word is set.
const HIGH: u64 = 0x1_0000_2000;
const LOW: u64 = 0x0000_2000;
/// The largest register the gate owes an arm.
const AT_CEILING: u64 = u32::MAX as u64;
/// The smallest register the gate refuses.
const ABOVE_CEILING: u64 = AT_CEILING + 1;

/// Each `Unsupported` syscall and the argument slots it binds as u32.
///
/// A slot is absent for one of two reasons:
///
/// - The arm reads that argument at its full 64-bit width. Every
///   `ipc_key`, `size` and `alignment` word is one. So are the 64-bit
///   `flags` of 332 and 362 and `sys_mmapper_map_shared_memory`'s
///   `addr`. So is the `sys_ppu_thread_*` `thread_id`, which names a
///   64-bit [`crate::ppu_thread::PpuThreadId`].
/// - The arm binds that argument as a signed 32-bit field with a
///   truncating cast. `sys_ppu_thread_set_priority`'s `prio` and
///   `sys_mmapper_allocate_shared_memory_ext`'s `entry_count` drop a
///   high word instead of a refusal.
///
/// Syscall 47 binds no u32 field, so it has no row.
const UNSUPPORTED_U32_SLOTS: &[(u64, &[usize])] = &[
    (syscall::SYS_PRX_LOAD_MODULE, &[0]),
    (syscall::SYS_PRX_START_MODULE, &[0, 2]),
    (syscall::SYS_PRX_STOP_MODULE, &[0, 2]),
    (syscall::SYS_PRX_UNLOAD_MODULE, &[0]),
    (syscall::SYS_PRX_REGISTER_MODULE, &[1]),
    (syscall::SYS_PRX_REGISTER_LIBRARY, &[0]),
    (syscall::SYS_PRX_GET_MODULE_LIST, &[1]),
    (syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER, &[0]),
    (syscall::MEMORY_CONTAINER_CREATE_324, &[0]),
    (syscall::MMAPPER_ALLOCATE_ADDRESS, &[3]),
    (syscall::MMAPPER_ALLOCATE_SHARED_MEMORY, &[3]),
    (syscall::MMAPPER_MAP_SHARED_MEMORY, &[1]),
    (syscall::MMAPPER_SEARCH_AND_MAP, &[0, 1, 3]),
    (syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER, &[4]),
    (syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT, &[2, 3, 5]),
    (syscall::PPU_THREAD_GET_PRIORITY, &[1]),
    (syscall::EVENT_PORT_CONNECT_LOCAL, &[0, 1]),
    (syscall::EVENT_PORT_CONNECT_IPC, &[0]),
    (syscall::EVENT_PORT_DISCONNECT, &[0]),
];

/// The slot count the table names, so a lost row fails the sweeps
/// below. A sweep that covers fewer slots is otherwise silent.
const GATED_SLOTS: usize = 26;

/// Calls `f` for every slot in the table, then checks the count
/// against [`GATED_SLOTS`].
fn for_each_slot(mut f: impl FnMut(u64, usize)) {
    let mut seen = 0;
    for (number, slots) in UNSUPPORTED_U32_SLOTS {
        for &slot in *slots {
            f(*number, slot);
            seen += 1;
        }
    }
    assert_eq!(seen, GATED_SLOTS, "table names a different slot count");
}

/// Every other slot stays zero. The gate precedes each arm's own
/// argument tests, so no row needs a well-formed companion argument.
fn args_with(slot: usize, value: u64) -> [u64; 8] {
    let mut args = [0u64; 8];
    args[slot] = value;
    args
}

fn dispatch_with(number: u64, args: [u64; 8]) -> (Lv2Host, Lv2Dispatch) {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::with_memory(cellgov_mem::GuestMemory::new(0x10000));
    let out = host.dispatch(
        Lv2Request::Unsupported { number, args },
        UnitId::new(0),
        &rt,
    );
    (host, out)
}

#[test]
fn every_unsupported_u32_slot_refuses_a_register_that_carries_high_bits() {
    for_each_slot(|number, slot| {
        let (host, out) = dispatch_with(number, args_with(slot, HIGH));
        assert_eq!(
            out,
            Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            "sc {number} arg {slot}"
        );
        assert_eq!(
            host.invariant_break_site_count("dispatch.arg_high_bits"),
            1,
            "sc {number} arg {slot}"
        );
    });
}

#[test]
fn every_unsupported_u32_slot_admits_a_register_whose_high_word_is_clear() {
    // Each arm answers this address on its own terms. Only the break
    // count separates a gate that admits the register from one that
    // refuses it.
    for_each_slot(|number, slot| {
        let (host, _) = dispatch_with(number, args_with(slot, LOW));
        assert_eq!(
            host.invariant_break_site_count("dispatch.arg_high_bits"),
            0,
            "sc {number} arg {slot}"
        );
    });
}

#[test]
fn every_unsupported_u32_slot_turns_over_exactly_at_the_u32_ceiling() {
    // LOW and HIGH sit far from the boundary, so a gate that refuses
    // one value too many -- or one too few -- still clears them both.
    for_each_slot(|number, slot| {
        let (host, _) = dispatch_with(number, args_with(slot, AT_CEILING));
        assert_eq!(
            host.invariant_break_site_count("dispatch.arg_high_bits"),
            0,
            "sc {number} arg {slot} at u32::MAX"
        );

        let (host, out) = dispatch_with(number, args_with(slot, ABOVE_CEILING));
        assert_eq!(
            out,
            Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            "sc {number} arg {slot} one past u32::MAX"
        );
        assert_eq!(
            host.invariant_break_site_count("dispatch.arg_high_bits"),
            1,
            "sc {number} arg {slot} one past u32::MAX"
        );
    });
}

#[test]
fn full_width_slots_answer_for_the_whole_register() {
    // sc 334's `addr`. A truncated 0x1_2000_0000 lands inside the
    // mapping window, so a truncating arm reaches the `mem_id` lookup
    // and answers CELL_ESRCH for the unknown handle.
    let (host, out) = dispatch_with(
        syscall::MMAPPER_MAP_SHARED_MEMORY,
        [0x1_2000_0000, 0, 0, 0, 0, 0, 0, 0],
    );
    assert_eq!(out, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
    assert_eq!(
        host.invariant_break_site_count("dispatch.mmapper_map_unknown_mem_id"),
        0
    );

    // sc 140's `ipc_key`. A truncated 0x1_0000_0000 reads as the zero
    // key, which the arm refuses before it counts the attempt.
    let (host, out) = dispatch_with(
        syscall::EVENT_PORT_CONNECT_IPC,
        [0, 0x1_0000_0000, 0, 0, 0, 0, 0, 0],
    );
    assert_eq!(out, Lv2Dispatch::immediate(errno::CELL_ESRCH.into()));
    assert_eq!(host.observability().event_port_ipc_connects.0, 1);
}

#[test]
fn a_ppu_thread_id_above_the_u32_ceiling_reaches_the_lookup() {
    // `sys_ppu_thread_join` binds its target at full width and
    // `PpuThreadId` holds 64 bits, so 47 and 48 resolve the same ids.
    // This register aliases the primary thread in its low word, so the
    // three readings differ:
    //
    // - A truncating read finds that thread and answers CELL_OK.
    // - A width gate answers CELL_EINVAL.
    // - A full-width lookup answers CELL_ESRCH for an id the table
    //   does not hold.
    use crate::host::test_support::seed_primary_ppu;
    let aliased = 0x1_0000_0000 | crate::ppu_thread::PpuThreadId::PRIMARY.raw();

    for (number, companion) in [
        (syscall::PPU_THREAD_GET_PRIORITY, 0x2000),
        (syscall::PPU_THREAD_SET_PRIORITY, 5),
    ] {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::with_memory(cellgov_mem::GuestMemory::new(0x10000));
        seed_primary_ppu(&mut host, UnitId::new(0));
        let mut args = [0u64; 8];
        args[0] = aliased;
        args[1] = companion;
        let out = host.dispatch(
            Lv2Request::Unsupported { number, args },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            out,
            Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            "sc {number}"
        );
        assert_eq!(
            host.invariant_break_site_count("dispatch.arg_high_bits"),
            0,
            "sc {number}"
        );
    }
}
