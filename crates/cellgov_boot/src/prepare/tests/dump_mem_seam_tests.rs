//! A `--dump-mem` line across two regions that sit end to end names
//! both regions.

use std::cell::RefCell;

use super::dump_boot_memory;
use crate::BootSink;
use cellgov_core::Runtime;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

const SIZE: usize = 0x1_0000;
const EDGE: u64 = SIZE as u64;

#[derive(Default)]
struct Notes(RefCell<Vec<String>>);

impl BootSink for Notes {
    fn note(&self, line: &str) {
        self.0.borrow_mut().push(line.to_string());
    }
    fn warn(&self, _line: &str) {}
    fn guest_text(&self, _text: &str) {}
}

/// The byte at `addr` beside the region boundary: its low byte, so each
/// byte of a dump names the address it came from.
fn byte_at(addr: u64) -> u8 {
    addr.to_le_bytes()[0]
}

/// The dump's hex text for the 32 bytes at `addr`.
fn hex_from(addr: u64) -> String {
    (addr..addr + 32)
        .map(|a| format!(" {:02x}", byte_at(a)))
        .collect()
}

fn dump(addr: u64) -> String {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, SIZE, "low", PageSize::Page64K),
        Region::new(EDGE, SIZE, "high", PageSize::Page64K),
    ])
    .expect("disjoint regions");
    let fill: Vec<u8> = (EDGE - 32..EDGE + 32).map(byte_at).collect();
    let range = ByteRange::new(GuestAddr::new(EDGE - 32), 64).expect("a small range");
    mem.apply_commit(range, &fill)
        .expect("both regions are writable");
    let rt = Runtime::new(mem, Budget::new(1), 1);
    let notes = Notes::default();
    dump_boot_memory(&rt, &[addr], &notes);
    let lines = notes.0.into_inner();
    assert_eq!(lines.len(), 1, "one line per address: {lines:?}");
    lines[0].clone()
}

#[test]
fn a_dump_across_the_seam_names_both_regions_and_every_byte() {
    assert_eq!(
        dump(EDGE - 16),
        format!("mem[0x{:x}] (low+high):{}", EDGE - 16, hex_from(EDGE - 16))
    );
}

#[test]
fn a_dump_in_one_region_names_that_region_alone() {
    assert_eq!(
        dump(EDGE),
        format!("mem[0x{EDGE:x}] (high):{}", hex_from(EDGE))
    );
}
