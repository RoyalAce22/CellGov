//! One rule per width binds the guest fields of every `Unsupported`
//! arm.
//!
//! - A u32 field refuses a register that carries a high word.
//! - An `int` field refuses a register that is no sign extension of
//!   its own low word.

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
/// - The arm binds that argument as an `int`, so it appears in
///   [`UNSUPPORTED_I32_SLOTS`] instead.
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

/// Each `Unsupported` syscall and the argument slots it binds as an
/// `int`. Each arm's own range test reads its field as signed.
/// Syscall 47 admits a negative `prio` under debug-or-root capability.
const UNSUPPORTED_I32_SLOTS: &[(u64, &[usize])] = &[
    (syscall::PPU_THREAD_SET_PRIORITY, &[1]),
    (syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT, &[4]),
];

/// The slot count the table names, so a lost row fails the sweeps
/// below. A sweep that covers fewer slots is otherwise silent.
const GATED_SLOTS: usize = 26;

/// [`GATED_SLOTS`] for the `int` table.
const GATED_I32_SLOTS: usize = 2;

/// Calls `f` for every slot in `table`, then checks the count against
/// `expected`.
fn for_each_slot_of(table: &[(u64, &[usize])], expected: usize, mut f: impl FnMut(u64, usize)) {
    let mut seen = 0;
    for (number, slots) in table {
        for &slot in *slots {
            f(*number, slot);
            seen += 1;
        }
    }
    assert_eq!(seen, expected, "table names a different slot count");
}

fn for_each_slot(f: impl FnMut(u64, usize)) {
    for_each_slot_of(UNSUPPORTED_U32_SLOTS, GATED_SLOTS, f);
}

fn for_each_i32_slot(f: impl FnMut(u64, usize)) {
    for_each_slot_of(UNSUPPORTED_I32_SLOTS, GATED_I32_SLOTS, f);
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
fn every_unsupported_i32_slot_refuses_a_register_that_is_no_sign_extension() {
    // 0x1_0000_0001 reads as 1 under a truncating cast. 0x8000_0000
    // wraps to i32::MIN. Neither register carries the value its low
    // word names.
    for probe in [0x1_0000_0001u64, 0x8000_0000] {
        for_each_i32_slot(|number, slot| {
            let (host, out) = dispatch_with(number, args_with(slot, probe));
            assert_eq!(
                out,
                Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
                "sc {number} arg {slot} = {probe:#x}"
            );
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_not_sign_extended"),
                1,
                "sc {number} arg {slot} = {probe:#x}"
            );
        });
    }
}

#[test]
fn every_unsupported_i32_slot_admits_a_sign_extended_register() {
    // Each arm answers these on its own terms, and 7 is inside both
    // range windows. Only the break count separates a gate that
    // admits the register from one that refuses it.
    for probe in [7u64, u64::MAX] {
        for_each_i32_slot(|number, slot| {
            let (host, _) = dispatch_with(number, args_with(slot, probe));
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_not_sign_extended"),
                0,
                "sc {number} arg {slot} = {probe:#x}"
            );
        });
    }
}

#[test]
fn every_unsupported_i32_slot_turns_over_exactly_at_the_i32_bounds() {
    // Neither bound is pinned above: the admitted probes sit far
    // inside the window and no refused probe pins the floor. These
    // four values straddle each bound by one.
    let admitted = [i32::MAX as u64, i32::MIN as i64 as u64];
    let refused = [i32::MAX as u64 + 1, (i32::MIN as i64 as u64) - 1];
    for (ok, bad) in admitted.into_iter().zip(refused) {
        for_each_i32_slot(|number, slot| {
            let (host, _) = dispatch_with(number, args_with(slot, ok));
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_not_sign_extended"),
                0,
                "sc {number} arg {slot} = {ok:#x}"
            );

            let (host, out) = dispatch_with(number, args_with(slot, bad));
            assert_eq!(
                out,
                Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
                "sc {number} arg {slot} = {bad:#x}"
            );
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_not_sign_extended"),
                1,
                "sc {number} arg {slot} = {bad:#x}"
            );
        });
    }
}

#[test]
fn an_entry_count_that_is_no_sign_extension_refuses_a_call_that_would_otherwise_succeed() {
    // The sweeps above zero every companion argument, so each arm
    // refuses their probes for a second reason. The sweep proves only
    // that the errno and the break site are the gate's. This call is
    // well formed, so a truncating read of 0x1_0000_0001 binds one
    // entry and mints a handle. `sys_ppu_thread_set_priority` has the
    // same witness beside its own arm.
    use cellgov_ps3_abi::lv2::memory::page_size;
    const KEY: u64 = 0x8000_4d49_4f32_3211;
    const ENTRIES: u64 = 0x4000;
    const MEM_ID_PTR: u64 = 0x9000;
    const SIZE_64K: u64 = 0x2_0000;

    let well_formed = [
        KEY,
        SIZE_64K,
        page_size::FLAG_64K,
        ENTRIES,
        1,
        MEM_ID_PTR,
        0,
        0,
    ];
    let (host, out) = dispatch_with(syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT, well_formed);
    assert!(
        matches!(out, Lv2Dispatch::Immediate { code: 0, .. }),
        "the call the gate must refuse has to succeed without it: {out:?}"
    );
    assert!(host.state.mmapper_ipc.contains_key(&KEY));

    let mut aliased = well_formed;
    aliased[4] = 0x1_0000_0001;
    let (host, out) = dispatch_with(syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT, aliased);
    assert_eq!(out, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
    assert_eq!(
        host.invariant_break_site_count("dispatch.arg_not_sign_extended"),
        1
    );
    assert!(host.state.mmapper_ipc.is_empty());
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
