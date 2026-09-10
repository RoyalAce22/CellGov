//! `sys_ppu_thread_set_priority`: range gate, id lookup, and the
//! stored value `sys_ppu_thread_get_priority` reads back.

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::request::Lv2Request;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::ppu_thread::{PPU_THREAD_PRIORITY_MAX, PPU_THREAD_PRIORITY_MIN_ROOT};
use cellgov_ps3_abi::lv2::{errno, syscall};

const PRIO_PTR: u32 = 0x2000;

fn src() -> UnitId {
    UnitId::new(0)
}

fn rt() -> FakeRuntime {
    FakeRuntime::with_memory(GuestMemory::new(0x10000))
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn set_priority(host: &mut Lv2Host, rt: &FakeRuntime, thread_id: u64, prio: i64) -> u64 {
    let mut args = [0u64; 8];
    args[0] = thread_id;
    args[1] = prio as u64;
    code_of(&host.dispatch(
        Lv2Request::Unsupported {
            number: syscall::PPU_THREAD_SET_PRIORITY,
            args,
        },
        src(),
        rt,
    ))
}

fn get_priority(host: &mut Lv2Host, rt: &FakeRuntime, thread_id: u64) -> u32 {
    let mut args = [0u64; 8];
    args[0] = thread_id;
    args[1] = u64::from(PRIO_PTR);
    let d = host.dispatch(
        Lv2Request::Unsupported {
            number: syscall::PPU_THREAD_GET_PRIORITY,
            args,
        },
        src(),
        rt,
    );
    assert_eq!(code_of(&d), 0);
    match &d {
        Lv2Dispatch::Immediate { effects, .. } => extract_write_u32(&effects[0]),
        _ => unreachable!(),
    }
}

#[test]
fn set_priority_is_read_back_by_get_priority() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let id = PpuThreadId::PRIMARY.raw();
    assert_eq!(get_priority(&mut host, &rt, id), 0);
    assert_eq!(set_priority(&mut host, &rt, id, 1001), 0);
    assert_eq!(get_priority(&mut host, &rt, id), 1001);
    assert_eq!(set_priority(&mut host, &rt, id, 0), 0);
    assert_eq!(get_priority(&mut host, &rt, id), 0);
}

#[test]
fn the_range_bounds_are_inclusive() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let id = PpuThreadId::PRIMARY.raw();
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MAX)),
        0
    );
    assert_eq!(
        get_priority(&mut host, &rt, id),
        PPU_THREAD_PRIORITY_MAX as u32
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MAX) + 1),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, -1),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MIN_ROOT)),
        u64::from(errno::CELL_EINVAL),
        "a user process does not get the root floor"
    );
    assert_eq!(
        get_priority(&mut host, &rt, id),
        PPU_THREAD_PRIORITY_MAX as u32
    );
}

#[test]
fn a_debug_or_root_process_may_go_down_to_the_root_floor() {
    // The priority floor is -512 under debug_or_root and 0 otherwise;
    // the 3071 ceiling does not move. The privileged widening below
    // zero has no public anchor.
    let rt = rt();
    let mut host = Lv2Host::new();
    host.set_control_flags1(cellgov_ps3_abi::format::sce::CTRL_FLAGS1_ROOT_MASK);
    seed_primary_ppu(&mut host, src());
    let id = PpuThreadId::PRIMARY.raw();
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MIN_ROOT)),
        0
    );
    // Stored as the two's-complement image `sys_ppu_thread_get_priority`
    // hands back, which the guest reads as a signed int.
    assert_eq!(
        get_priority(&mut host, &rt, id),
        PPU_THREAD_PRIORITY_MIN_ROOT as u32
    );
    assert_eq!(
        set_priority(
            &mut host,
            &rt,
            id,
            i64::from(PPU_THREAD_PRIORITY_MIN_ROOT) - 1
        ),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MAX) + 1),
        u64::from(errno::CELL_EINVAL)
    );
}

#[test]
fn a_range_error_precedes_the_id_lookup_and_an_unknown_id_is_esrch() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let unknown = 0x7fff_ffff;
    assert_eq!(
        set_priority(&mut host, &rt, unknown, 5000),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, unknown, 5),
        u64::from(errno::CELL_ESRCH)
    );
}

#[test]
fn a_prio_register_that_is_no_sign_extension_is_refused() {
    // `prio` is an `int`, so the guest ABI sign-extends a negative
    // value and the arm reads the whole register. A register that
    // reproduces no `int` names no priority. A read of its low word
    // would accept 0x1_0000_0007 as priority 7.
    //
    // Both registers below answer CELL_EINVAL, so only the break count
    // says which gate refused each one. The stored priority is a value
    // no refusal writes, so the read-back separates a refusal from a
    // store of the default.
    const GATE: &str = "dispatch.arg_not_sign_extended";
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let id = PpuThreadId::PRIMARY.raw();
    assert_eq!(set_priority(&mut host, &rt, id, 1001), 0);

    assert_eq!(
        set_priority(&mut host, &rt, id, -2),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(
        host.invariant_break_site_count(GATE),
        0,
        "a sign-extended -2 reaches the range window"
    );

    assert_eq!(
        set_priority(&mut host, &rt, id, 0x1_0000_0000 + 7),
        u64::from(errno::CELL_EINVAL)
    );
    assert_eq!(host.invariant_break_site_count(GATE), 1);

    assert_eq!(get_priority(&mut host, &rt, id), 1001);
}
