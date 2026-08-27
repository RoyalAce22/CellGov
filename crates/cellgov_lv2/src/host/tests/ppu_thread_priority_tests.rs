//! `sys_ppu_thread_set_priority`: range gate, id lookup, and the
//! stored value `sys_ppu_thread_get_priority` reads back.

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::request::Lv2Request;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::sys_ppu_thread::{PPU_THREAD_PRIORITY_MAX, PPU_THREAD_PRIORITY_MIN_ROOT};
use cellgov_ps3_abi::{cell_errors, syscall};

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
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, -1),
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MIN_ROOT)),
        u64::from(cell_errors::CELL_EINVAL),
        "a user process does not get the root floor"
    );
    assert_eq!(
        get_priority(&mut host, &rt, id),
        PPU_THREAD_PRIORITY_MAX as u32
    );
}

#[test]
fn a_debug_or_root_process_may_go_down_to_the_root_floor() {
    // RPCS3 sys_ppu_thread.cpp sys_ppu_thread_set_priority: the floor
    // is -512 under debug_or_root, 0 otherwise; the ceiling does not move.
    let rt = rt();
    let mut host = Lv2Host::new();
    host.set_control_flags1(cellgov_ps3_abi::sce::CTRL_FLAGS1_ROOT_MASK);
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
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, id, i64::from(PPU_THREAD_PRIORITY_MAX) + 1),
        u64::from(cell_errors::CELL_EINVAL)
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
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(
        set_priority(&mut host, &rt, unknown, 5),
        u64::from(cell_errors::CELL_ESRCH)
    );
}

#[test]
fn only_the_low_word_of_the_prio_argument_is_read() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let id = PpuThreadId::PRIMARY.raw();
    // A sign-extended 32-bit value in a 64-bit register is what the
    // guest ABI hands over for a negative s32.
    assert_eq!(
        set_priority(&mut host, &rt, id, -2),
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(set_priority(&mut host, &rt, id, 0x1_0000_0000 + 7), 0);
    assert_eq!(get_priority(&mut host, &rt, id), 7);
}
