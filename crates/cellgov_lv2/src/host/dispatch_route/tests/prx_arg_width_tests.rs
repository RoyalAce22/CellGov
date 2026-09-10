//! One rule binds the u32 guest fields of every `_sys_prx_*` arm.

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

/// Each `_sys_prx_*` syscall and the argument slots it binds as u32.
const PRX_U32_SLOTS: &[(u64, &[usize])] = &[
    (syscall::SYS_PRX_LOAD_MODULE, &[0]),
    (syscall::SYS_PRX_START_MODULE, &[0, 2]),
    (syscall::SYS_PRX_STOP_MODULE, &[0, 2]),
    (syscall::SYS_PRX_UNLOAD_MODULE, &[0]),
    (syscall::SYS_PRX_REGISTER_MODULE, &[1]),
    (syscall::SYS_PRX_REGISTER_LIBRARY, &[0]),
    (syscall::SYS_PRX_GET_MODULE_LIST, &[1]),
    (syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER, &[0]),
];

/// Arg 0 carries sc 494's fill-list bit, which the other arms read as
/// a module id or a path pointer of 2.
fn args_with(slot: usize, value: u64) -> [u64; 8] {
    let mut args = [0u64; 8];
    args[0] = cellgov_ps3_abi::lv2::prx::get_module_list_option::FLAG_FILL_LIST;
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
fn every_prx_u32_slot_refuses_a_register_that_carries_high_bits() {
    for (number, slots) in PRX_U32_SLOTS {
        for &slot in *slots {
            let (host, out) = dispatch_with(*number, args_with(slot, HIGH));
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
        }
    }
}

#[test]
fn every_prx_u32_slot_admits_a_register_whose_high_word_is_clear() {
    // The control for the gate above. Each arm answers this address on
    // its own terms. Only the break count separates a gate that let the
    // register through from one that refused it.
    for (number, slots) in PRX_U32_SLOTS {
        for &slot in *slots {
            let (host, _) = dispatch_with(*number, args_with(slot, LOW));
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_high_bits"),
                0,
                "sc {number} arg {slot}"
            );
        }
    }
}

#[test]
fn every_prx_u32_slot_turns_over_exactly_at_the_u32_ceiling() {
    // LOW and HIGH sit far from the boundary, so a gate that refused
    // one value too many -- or one too few -- still clears them both.
    for (number, slots) in PRX_U32_SLOTS {
        for &slot in *slots {
            let (host, _) = dispatch_with(*number, args_with(slot, AT_CEILING));
            assert_eq!(
                host.invariant_break_site_count("dispatch.arg_high_bits"),
                0,
                "sc {number} arg {slot} at u32::MAX"
            );

            let (host, out) = dispatch_with(*number, args_with(slot, ABOVE_CEILING));
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
        }
    }
}
