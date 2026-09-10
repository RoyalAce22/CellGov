//! Fault gate over the `sys_*_attribute_t` block the sync-object
//! create dispatches read.
//!
//! No corpus trace presents an attribute block that straddles the end
//! of a mapped region. The CELL_EFAULT these tests pin is CellGov's
//! model of the copy-in rather than a witnessed kernel answer.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::sync::event_flag_attribute;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    seed_primary_ppu, valid_sync_attr_bytes, FakeRuntime, SYNC_ATTR_SIZE,
};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

/// Size of the guest arena that fixes every placement in this file.
const MEM_SIZE: usize = 0x10000;

/// Bytes of the attribute the straddling placement leaves unmapped.
const STRADDLE_TAIL: usize = 8;

/// One past the highest attribute byte either create dispatch reads.
///
/// The event flag validates `type` last, and the semaphore stops
/// lower.
const VALIDATED_PREFIX: usize = event_flag_attribute::TYPE_OFFSET + 4;

// The straddling placement leaves a tail unmapped while its mapped
// prefix still carries every validated field, so the tail alone can
// decide the call.
const _: () = assert!(STRADDLE_TAIL > 0 && SYNC_ATTR_SIZE - STRADDLE_TAIL >= VALIDATED_PREFIX);

/// Attribute address whose last [`STRADDLE_TAIL`] bytes fall outside
/// the arena.
const STRADDLING_ATTR_PTR: u32 = (MEM_SIZE - (SYNC_ATTR_SIZE - STRADDLE_TAIL)) as u32;

/// Highest attribute address whose whole struct still fits the arena.
const LAST_FITTING_ATTR_PTR: u32 = (MEM_SIZE - SYNC_ATTR_SIZE) as u32;

/// Builds a [`MEM_SIZE`]-byte arena that holds the first `mapped`
/// bytes of [`valid_sync_attr_bytes`] at `attr_ptr`.
fn runtime_with_attr(attr_ptr: u32, mapped: usize) -> FakeRuntime {
    let mut mem = GuestMemory::new(MEM_SIZE);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(attr_ptr)), mapped as u64).unwrap(),
        &valid_sync_attr_bytes()[..mapped],
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

/// The arena the fault tests use, which maps every attribute byte
/// except the trailing `name` bytes.
fn straddling_runtime() -> FakeRuntime {
    runtime_with_attr(STRADDLING_ATTR_PTR, SYNC_ATTR_SIZE - STRADDLE_TAIL)
}

fn seeded_host() -> (Lv2Host, UnitId) {
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    (host, src)
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn semaphore_create(
    host: &mut Lv2Host,
    src: UnitId,
    rt: &FakeRuntime,
    attr_ptr: u32,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr,
            initial: 0,
            max: 1,
        },
        src,
        rt,
    )
}

fn event_flag_create(
    host: &mut Lv2Host,
    src: UnitId,
    rt: &FakeRuntime,
    attr_ptr: u32,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr,
            init: 0,
        },
        src,
        rt,
    )
}

#[test]
fn the_attribute_struct_is_thirty_two_bytes() {
    assert_eq!(SYNC_ATTR_SIZE, 32);
}

#[test]
fn semaphore_create_faults_when_the_attribute_name_is_unmapped() {
    let (mut host, src) = seeded_host();
    let rt = straddling_runtime();
    let r = semaphore_create(&mut host, src, &rt, STRADDLING_ATTR_PTR);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
}

#[test]
fn event_flag_create_faults_when_the_attribute_name_is_unmapped() {
    let (mut host, src) = seeded_host();
    let rt = straddling_runtime();
    let r = event_flag_create(&mut host, src, &rt, STRADDLING_ATTR_PTR);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
}

#[test]
fn the_last_attribute_that_fits_the_arena_creates_both_objects() {
    let (mut host, src) = seeded_host();
    let rt = runtime_with_attr(LAST_FITTING_ATTR_PTR, SYNC_ATTR_SIZE);
    let sem = semaphore_create(&mut host, src, &rt, LAST_FITTING_ATTR_PTR);
    assert_eq!(code_of(&sem), 0);
    let flag = event_flag_create(&mut host, src, &rt, LAST_FITTING_ATTR_PTR);
    assert_eq!(code_of(&flag), 0);
}
