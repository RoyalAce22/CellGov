//! ExecutionContext views over committed memory, reservations, and syscall-return plumbing.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::{ReservationTable, ReservedLine};

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).unwrap()
}

#[test]
fn context_exposes_committed_memory() {
    let mut mem = GuestMemory::new(16);
    mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
    let ctx = ExecutionContext::new(&mem);
    let bytes = ctx.memory().read(range(0, 4)).unwrap();
    assert_eq!(bytes, &[1, 2, 3, 4]);
}

#[test]
fn context_is_copy() {
    let mem = GuestMemory::new(8);
    let ctx = ExecutionContext::new(&mem);
    let copy = ctx;
    assert_eq!(ctx.memory().size(), copy.memory().size());
}

#[test]
fn reservation_held_is_false_without_view() {
    let mem = GuestMemory::new(8);
    let ctx = ExecutionContext::new(&mem);
    assert!(!ctx.reservation_held(UnitId::new(0)));
    assert!(!ctx.reservation_held(UnitId::new(7)));
}

#[test]
fn reservation_held_reads_installed_view() {
    let mem = GuestMemory::new(8);
    let mut table = ReservationTable::new();
    table.insert_or_replace(UnitId::new(3), ReservedLine::containing(0x1000));
    let ctx = ExecutionContext::new(&mem).with_reservations(&table);
    assert!(ctx.reservation_held(UnitId::new(3)));
    assert!(!ctx.reservation_held(UnitId::new(4)));
}

#[test]
fn with_reservations_preserves_other_fields() {
    let mem = GuestMemory::new(8);
    let received = [7u32, 9];
    let writes: [(u8, u64); 1] = [(13, 0xfeedface)];
    let ctx = ExecutionContext::with_syscall_return_and_regs(&mem, &received, 42, &writes);
    let table = ReservationTable::new();
    let ctx = ctx.with_reservations(&table);
    assert_eq!(ctx.received_messages(), &[7, 9]);
    assert_eq!(ctx.syscall_return(), Some(42));
    assert_eq!(ctx.register_writes(), &[(13, 0xfeedface)]);
}
