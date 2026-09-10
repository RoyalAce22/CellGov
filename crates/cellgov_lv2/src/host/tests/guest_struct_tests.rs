//! Field reads land on the byte the offset names. An unmapped range
//! refuses; no read falls on a neighbouring address.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
#[cfg(debug_assertions)]
use cellgov_time::GuestTicks;

use crate::host::guest_struct::{read_be_u32, read_be_u64, GuestStruct};
use crate::host::test_support::FakeRuntime;
#[cfg(debug_assertions)]
use crate::host::Lv2Runtime;

/// No field value equals the same field read one offset over, so a
/// mis-indexed accessor cannot pass.
const BLOB: [u8; 12] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC,
];

fn runtime_holding_blob() -> FakeRuntime {
    let mut mem = GuestMemory::new(64);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(8), BLOB.len() as u64).unwrap(),
        &BLOB,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

#[test]
fn fields_read_big_endian_at_their_offset() {
    let s = GuestStruct::new(&BLOB);
    assert_eq!(s.u16_at(0), 0x1122);
    assert_eq!(s.u16_at(1), 0x2233);
    assert_eq!(s.u32_at(0), 0x1122_3344);
    assert_eq!(s.u32_at(4), 0x5566_7788);
    assert_eq!(s.u64_at(0), 0x1122_3344_5566_7788);
    assert_eq!(s.u64_at(4), 0x5566_7788_99AA_BBCC);
}

#[test]
fn read_takes_the_struct_bytes_at_the_address() {
    let rt = runtime_holding_blob();
    let s = GuestStruct::read(&rt, 8, BLOB.len()).unwrap();
    assert_eq!(s.u32_at(0), 0x1122_3344);
    assert_eq!(s.u64_at(4), 0x5566_7788_99AA_BBCC);
}

#[test]
fn read_refuses_an_unmapped_range() {
    let rt = runtime_holding_blob();
    assert!(GuestStruct::read(&rt, 61, 4).is_none());
    assert!(read_be_u32(&rt, 61).is_none());
    assert!(read_be_u64(&rt, 57).is_none());
}

#[test]
fn scalar_readers_take_the_word_at_the_address() {
    let rt = runtime_holding_blob();
    assert_eq!(read_be_u32(&rt, 8), Some(0x1122_3344));
    assert_eq!(read_be_u32(&rt, 12), Some(0x5566_7788));
    assert_eq!(read_be_u64(&rt, 8), Some(0x1122_3344_5566_7788));
}

#[test]
#[should_panic(expected = "index out of bounds")]
fn a_field_past_the_end_panics() {
    GuestStruct::new(&BLOB).u32_at(9);
}

fn runtime_ending_at_blob() -> FakeRuntime {
    let mut mem = GuestMemory::new(8 + BLOB.len());
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(8), BLOB.len() as u64).unwrap(),
        &BLOB,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

#[test]
fn a_scalar_reader_asks_for_exactly_its_own_width() {
    let rt = runtime_ending_at_blob();
    // Both words end on the last mapped byte. A wider request answers
    // None here. A narrower one runs its accessor off the end of the
    // slice it gets back.
    assert_eq!(read_be_u32(&rt, 16), Some(0x99AA_BBCC));
    assert_eq!(read_be_u64(&rt, 12), Some(0x5566_7788_99AA_BBCC));
    assert!(read_be_u32(&rt, 17).is_none());
    assert!(read_be_u64(&rt, 13).is_none());
}

/// Breaks the `read_committed` contract: answers four bytes for any
/// `len`.
#[cfg(debug_assertions)]
struct ShortReader {
    bytes: [u8; 4],
}

#[cfg(debug_assertions)]
impl Lv2Runtime for ShortReader {
    fn read_committed(&self, _addr: u64, _len: usize) -> Option<&[u8]> {
        Some(&self.bytes[..])
    }

    fn current_tick(&self) -> GuestTicks {
        GuestTicks::ZERO
    }

    fn read_committed_until(&self, _addr: u64, _max_len: usize, _terminator: u8) -> Option<&[u8]> {
        None
    }

    fn writable(&self, _addr: u64, _len: usize) -> bool {
        true
    }

    fn committed_overlap_end(&self, _addr: u64, _size: u64) -> Option<u64> {
        None
    }
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "must carry exactly len bytes")]
fn a_short_answer_from_the_runtime_is_named() {
    let rt = ShortReader { bytes: [0; 4] };
    let _ = GuestStruct::read(&rt, 0, 8);
}
