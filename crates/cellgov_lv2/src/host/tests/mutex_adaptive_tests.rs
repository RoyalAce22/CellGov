//! The `adaptive` word of `sys_mutex_attribute_t` at create, and the
//! fault gate over the block it sits in.
//!
//! Every test but the last seeds protocol, recursive and pshared with
//! values the create accepts, so the adaptive word and the mapped
//! length alone decide the outcome.

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

const BREAK_SITE: &str = "mutex.adaptive_out_of_range";

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
    create_from(&attr_bytes(adaptive)[..mapped])
}

fn create_from(attr: &[u8]) -> (Lv2Host, Lv2Dispatch) {
    let mut host = Lv2Host::new();
    let (rt, attr_ptr) = runtime_with_attr(attr);
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
fn an_out_of_range_adaptive_word_is_named() {
    let (host, r) = create(0x1234, ATTR_LEN);
    assert_eq!(code_of(&r), 0);
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 1);
}

#[test]
fn a_zero_adaptive_word_is_named() {
    // A memset-zero block reaches this check only when the three
    // words before it are set, so zero here is a caller that skipped
    // one field rather than a caller that skipped the struct.
    let (host, r) = create(0, ATTR_LEN);
    assert_eq!(code_of(&r), 0);
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 1);
}

#[test]
fn the_adaptive_member_creates_the_mutex_without_a_break() {
    let (host, r) = create(SYS_SYNC_ADAPTIVE, ATTR_LEN);
    assert_eq!(code_of(&r), 0);
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 0);
}

#[test]
fn the_not_adaptive_member_creates_the_mutex_without_a_break() {
    let (host, r) = create(SYS_SYNC_NOT_ADAPTIVE, ATTR_LEN);
    assert_eq!(code_of(&r), 0);
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 0);
}

#[test]
fn a_create_faults_when_the_adaptive_word_is_unmapped() {
    let (host, r) = create(SYS_SYNC_NOT_ADAPTIVE, mutex_attribute::ADAPTIVE_OFFSET);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 0);
}

#[test]
fn a_create_faults_when_the_attribute_name_is_unmapped() {
    // The name field decides nothing, so a gate that stopped at the
    // words the create validates would accept this block. The two
    // sibling creates gate their whole struct for the same reason.
    let (host, r) = create(SYS_SYNC_NOT_ADAPTIVE, ATTR_LEN - 8);
    assert_eq!(code_of(&r), errno::CELL_EFAULT.into());
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 0);
}

#[test]
fn a_create_refused_by_an_earlier_word_records_no_adaptive_break() {
    // The ladder stops at its first refusal, so a create the pshared
    // word refuses mints nothing, and a break here would witness a
    // mutex that does not exist.
    let mut attr = attr_bytes(0x1234);
    let pshared = mutex_attribute::PSHARED_OFFSET;
    attr[pshared..pshared + 4].copy_from_slice(&0u32.to_be_bytes());
    let (host, r) = create_from(&attr);
    assert_eq!(code_of(&r), errno::CELL_EINVAL.into());
    assert_eq!(host.invariant_break_site_count(BREAK_SITE), 0);
}
