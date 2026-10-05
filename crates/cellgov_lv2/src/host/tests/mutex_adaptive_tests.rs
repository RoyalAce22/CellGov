//! The `adaptive` word of `sys_mutex_attribute_t` at create, and the
//! fault gate over the block it sits in.
//!
//! Every test seeds protocol, recursive and pshared with values the
//! create accepts, so the adaptive word and the mapped length alone
//! decide the outcome.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::sync::{
    mutex_attribute, SYS_SYNC_ADAPTIVE, SYS_SYNC_FIFO, SYS_SYNC_NOT_ADAPTIVE,
    SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE,
};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{seed_primary_ppu, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

/// Bytes the create gates on.
const ATTR_LEN: usize = mutex_attribute::SIZE as usize;

const MEM_SIZE: usize = 0x10000;

fn attr_bytes(adaptive: u32) -> [u8; ATTR_LEN] {
    let mut attr = [0u8; ATTR_LEN];
    let put = |attr: &mut [u8; ATTR_LEN], off: usize, word: u32| {
        attr[off..off + 4].copy_from_slice(&word.to_be_bytes());
    };
    put(&mut attr, mutex_attribute::PROTOCOL_OFFSET, SYS_SYNC_FIFO);
    put(
        &mut attr,
        mutex_attribute::RECURSIVE_OFFSET,
        SYS_SYNC_NOT_RECURSIVE,
    );
    put(
        &mut attr,
        mutex_attribute::PSHARED_OFFSET,
        SYS_SYNC_NOT_PROCESS_SHARED,
    );
    put(&mut attr, mutex_attribute::ADAPTIVE_OFFSET, adaptive);
    attr
}

/// Guest memory whose last `attr.len()` bytes are `attr`, plus the
/// address they start at.
///
/// An `attr` shorter than [`ATTR_LEN`] leaves the rest of the
/// attribute past the end of the arena, where a read reports the
/// range unmapped.
fn runtime_with_attr(attr: &[u8]) -> (FakeRuntime, u32) {
    let attr_ptr = (MEM_SIZE - attr.len()) as u32;
    let mut mem = GuestMemory::new(MEM_SIZE);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(attr_ptr as u64), attr.len() as u64).unwrap(),
        attr,
    )
    .unwrap();
    (FakeRuntime::with_memory(mem), attr_ptr)
}

fn create(adaptive: u32, mapped: usize) -> (Lv2Host, Lv2Dispatch) {
    let mut host = Lv2Host::new();
    let (rt, attr_ptr) = runtime_with_attr(&attr_bytes(adaptive)[..mapped]);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let dispatched = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr,
        },
        src,
        &rt,
    );
    (host, dispatched)
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn every_adaptive_word_creates_the_mutex_without_a_break() {
    for adaptive in [0, 0x1234, SYS_SYNC_ADAPTIVE, SYS_SYNC_NOT_ADAPTIVE] {
        let (host, r) = create(adaptive, ATTR_LEN);
        assert_eq!(code_of(&r), 0, "adaptive {adaptive:#x}");
        assert_eq!(host.mutexes().len(), 1, "adaptive {adaptive:#x}");
        assert_eq!(
            host.observability().invariant_break_count,
            0,
            "adaptive {adaptive:#x}"
        );
    }
}

#[test]
fn a_create_faults_when_the_adaptive_word_is_unmapped() {
    let (host, r) = create(SYS_SYNC_NOT_ADAPTIVE, mutex_attribute::ADAPTIVE_OFFSET);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    assert_eq!(host.mutexes().len(), 0);
}

#[test]
fn a_create_faults_when_the_attribute_name_is_unmapped() {
    // The name field decides nothing, so a gate that stopped at the
    // words the create validates would accept this block. The two
    // sibling creates gate their whole struct for the same reason.
    let (host, r) = create(SYS_SYNC_NOT_ADAPTIVE, ATTR_LEN - 8);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    assert_eq!(host.mutexes().len(), 0);
}
