//! FakeRuntime harness behavior: committed-memory reads, terminator-bounded string reads, and writable-override precedence.

use super::*;

#[test]
fn fake_runtime_reads_committed_memory() {
    let rt = FakeRuntime::new(256);
    assert!(rt.read_committed(0, 4).is_some());
    assert!(rt.read_committed(252, 4).is_some());
    assert!(rt.read_committed(253, 4).is_none());
    assert!(rt.read_committed(0, 0).is_some());
}

#[test]
fn read_committed_until_terminator_at_first_byte_returns_empty_prefix() {
    let mut mem = GuestMemory::new(16);
    mem.apply_commit(ByteRange::new(GuestAddr::new(0), 1).unwrap(), &[0u8])
        .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    assert_eq!(rt.read_committed_until(0, 8, 0), Some(&[][..]));
}

#[test]
fn read_committed_until_no_terminator_within_max_len_returns_none() {
    let mut mem = GuestMemory::new(16);
    mem.apply_commit(ByteRange::new(GuestAddr::new(0), 4).unwrap(), b"abcd")
        .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    assert_eq!(rt.read_committed_until(0, 4, 0), None);
}

#[test]
fn read_committed_until_max_len_past_end_clamps_and_returns_none_when_terminator_absent() {
    let rt = FakeRuntime::new(16);
    assert_eq!(rt.read_committed_until(0, 100, 0xFF), None);
}

#[test]
fn read_committed_until_start_at_end_returns_none() {
    let rt = FakeRuntime::new(16);
    assert_eq!(rt.read_committed_until(16, 4, 0), None);
}

#[test]
fn writable_at_takes_precedence_over_writable_override() {
    let rt = FakeRuntime::new(256)
        .with_writable_override(true)
        .with_writable_at(0x40, false);
    assert!(!rt.writable(0x40, 4), "per-addr override must beat global");
    assert!(
        rt.writable(0x80, 4),
        "addresses with no per-addr entry must fall through to writable_override"
    );
}

#[test]
fn writable_at_miss_with_override_some_returns_override() {
    let rt = FakeRuntime::new(256).with_writable_override(false);
    assert!(
        !rt.writable(0x40, 4),
        "writable_override=false beats the otherwise-valid bounds check"
    );
}

#[test]
fn writable_no_override_falls_through_to_bounds_check() {
    let rt = FakeRuntime::new(256);
    assert!(rt.writable(0, 4), "in-bounds + no override -> writable");
    assert!(
        !rt.writable(254, 4),
        "out-of-bounds + no override -> unwritable"
    );
    assert!(
        !rt.writable(u64::MAX, 4),
        "addr+len overflow + no override -> unwritable"
    );
}
