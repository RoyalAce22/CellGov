//! The reservation granule held to the `ppu_granule_nan` capture from
//! the reference console. One thread holds a reservation on a
//! 128-byte-aligned line while a second stores one word at line + 64,
//! line + 128 or line + 0x1000; the first then tries `stwcx.`. The
//! probe runs 32 trials per offset.

use super::*;

const LINE: u64 = 0x1_0000;
const OFFSETS: [u64; 3] = [64, 128, 0x1000];

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_granule_nan/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

#[test]
fn another_units_store_breaks_the_reservation_exactly_where_it_did_on_the_console() {
    let w = words();
    let trials = w[44];
    assert_eq!(trials, 32, "trials per offset");
    let holder = UnitId::new(1);
    let storer = UnitId::new(2);
    for (k, &off) in OFFSETS.iter().enumerate() {
        let (succeeded, failed, timeouts) = (w[35 + 3 * k], w[36 + 3 * k], w[37 + 3 * k]);
        assert_eq!(timeouts, 0, "offset {off:#x}: every store arrived");
        let console_breaks = if failed == trials && succeeded == 0 {
            true
        } else if succeeded == trials && failed == 0 {
            false
        } else {
            panic!("offset {off:#x}: the console was split, {succeeded} kept / {failed} broke");
        };
        let mut table = ReservationTable::new();
        table.insert_or_replace(holder, ReservedLine::containing(LINE));
        table.clear_covering(LINE + off, 4, Some(storer));
        assert_eq!(!table.is_held_by(holder), console_breaks, "offset {off:#x}");
    }
}
