//! Tests of `GuestMemory::rewind_provisional_reads`.

use super::*;

fn mem() -> GuestMemory {
    GuestMemory::new(0x100)
}

#[test]
fn a_rewind_takes_back_the_reads_noted_after_the_count() {
    let mem = mem();
    mem.note_provisional_read(0x10, 4);
    let count = mem.provisional_read_count();
    mem.note_provisional_read(0x10, 4);
    mem.note_provisional_read(0x20, 8);

    mem.rewind_provisional_reads(count);

    assert_eq!(mem.provisional_read_count(), 1);
    assert_eq!(
        mem.drain_provisional_reads(),
        [ProvisionalRead {
            addr: 0x10,
            len: 4,
            hits: 1,
        }]
    );
}

#[test]
fn a_rewind_to_the_current_count_changes_nothing() {
    let mem = mem();
    mem.note_provisional_read(0x10, 4);
    mem.rewind_provisional_reads(mem.provisional_read_count());
    assert_eq!(mem.provisional_read_count(), 1);
    assert_eq!(mem.drain_provisional_reads().len(), 1);
}

#[test]
fn a_rewind_takes_back_no_drained_read() {
    let mem = mem();
    mem.note_provisional_read(0x10, 4);
    mem.drain_provisional_reads();
    mem.rewind_provisional_reads(0);
    assert_eq!(mem.provisional_read_count(), 1, "the drained read stays");
}
