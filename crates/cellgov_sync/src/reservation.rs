//! Atomic reservation table shared across PPU and SPU.
//!
//! Models PPU `lwarx`/`ldarx` + `stwcx`/`stdcx` and SPU `MFC_GETLLAR` +
//! `MFC_PUTLLC` over a 128-byte cache-line granule. The commit
//! pipeline clears every entry whose line overlaps a committed write;
//! a conditional store succeeds only if the unit's entry is still
//! present at commit time. The table holds the global half of the
//! verdict, ANDed with the unit's local reservation register.
//!
//! Keys are canonical line addresses (low 7 bits zero). Callers pass
//! byte-granular addresses; the table canonicalizes on insert.

use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// Cache-line granule in bytes (Cell BE, PPU and SPU).
pub const RESERVATION_LINE_BYTES: u64 = 128;

/// 128-byte-aligned guest address. Low 7 bits are always zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReservedLine {
    addr: u64,
}

impl ReservedLine {
    /// Canonical line containing `byte_addr`.
    #[inline]
    pub const fn containing(byte_addr: u64) -> Self {
        Self {
            addr: byte_addr & !(RESERVATION_LINE_BYTES - 1),
        }
    }

    /// Canonical (128-byte-aligned) line address.
    #[inline]
    pub const fn addr(self) -> u64 {
        self.addr
    }

    /// Inclusive last byte of this line. Saturates on overflow.
    #[inline]
    pub const fn end_inclusive(self) -> u64 {
        self.addr.saturating_add(RESERVATION_LINE_BYTES - 1)
    }

    /// Whether this line overlaps `[start, start + len)`. Zero-length
    /// ranges never overlap. Uses saturating arithmetic; oversize
    /// writes are rejected upstream.
    #[inline]
    pub const fn overlaps_range(self, start: u64, len: u64) -> bool {
        if len == 0 {
            return false;
        }
        let write_end = start.saturating_add(len - 1);
        let line_end = self.end_inclusive();
        start <= line_end && self.addr <= write_end
    }
}

/// Committed atomic-reservation state, at most one entry per unit. A
/// second `insert_or_replace` for the same unit drops the prior entry
/// (matches PPC/SPU ABI: a second reserve invalidates the first).
#[derive(Debug, Clone, Default)]
pub struct ReservationTable {
    entries: BTreeMap<UnitId, ReservedLine>,
}

impl ReservationTable {
    /// Construct an empty table.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of units holding a reservation.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether any unit holds a reservation.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert or replace `unit`'s entry, returning the prior value.
    #[inline]
    pub fn insert_or_replace(&mut self, unit: UnitId, line: ReservedLine) -> Option<ReservedLine> {
        self.entries.insert(unit, line)
    }

    /// Drop `unit`'s entry. Returns `Some` iff an entry was present.
    #[inline]
    pub fn remove_if_present(&mut self, unit: UnitId) -> Option<ReservedLine> {
        self.entries.remove(&unit)
    }

    /// Read `unit`'s entry without mutating the table.
    #[inline]
    pub fn get(&self, unit: UnitId) -> Option<ReservedLine> {
        self.entries.get(&unit).copied()
    }

    /// Committed-state half of the conditional-store verdict; the
    /// local-reservation-register check lives on the unit.
    #[inline]
    pub fn is_held_by(&self, unit: UnitId) -> bool {
        self.entries.contains_key(&unit)
    }

    /// Iterate held reservations in unit-id order.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, ReservedLine)> + '_ {
        self.entries.iter().map(|(u, l)| (*u, *l))
    }

    /// Drop every entry whose line overlaps `[addr, addr + len)`.
    /// Returns the count dropped. O(n) over entries; short-circuits on
    /// empty table or zero-length writes.
    pub fn clear_covering(&mut self, addr: u64, len: u64) -> usize {
        if self.entries.is_empty() || len == 0 {
            return 0;
        }
        let before = self.entries.len();
        self.entries
            .retain(|_, line| !line.overlaps_range(addr, len));
        before - self.entries.len()
    }

    /// FNV-1a hash over `(unit_id, line_addr)` pairs in unit-id order.
    /// Empty table hashes to the FNV-1a empty-input value.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (unit, line) in self.entries.iter() {
            hasher.write(&unit.raw().to_le_bytes());
            hasher.write(&line.addr().to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: u64) -> UnitId {
        UnitId::new(id)
    }

    #[test]
    fn containing_canonicalizes_to_line_start() {
        for byte in [0u64, 1, 7, 63, 127] {
            assert_eq!(ReservedLine::containing(byte).addr(), 0);
        }
        for byte in [128u64, 129, 191, 255] {
            assert_eq!(ReservedLine::containing(byte).addr(), 128);
        }
        for byte in [256u64, 383] {
            assert_eq!(ReservedLine::containing(byte).addr(), 256);
        }
    }

    #[test]
    fn overlaps_range_zero_length_never_overlaps() {
        let line = ReservedLine::containing(0x1000);
        assert!(!line.overlaps_range(0x1000, 0));
        assert!(!line.overlaps_range(0x1080, 0));
    }

    #[test]
    fn overlaps_range_detects_write_inside_line() {
        let line = ReservedLine::containing(0x1000);
        assert!(line.overlaps_range(0x1000, 4));
        assert!(line.overlaps_range(0x1040, 8));
        assert!(line.overlaps_range(0x107C, 4));
    }

    #[test]
    fn overlaps_range_detects_write_spanning_line() {
        let line = ReservedLine::containing(0x1080);
        assert!(line.overlaps_range(0x1070, 32));
    }

    #[test]
    fn overlaps_range_rejects_non_adjacent_lines() {
        let line = ReservedLine::containing(0x1000);
        assert!(!line.overlaps_range(0x1080, 4));
        assert!(!line.overlaps_range(0x1100, 128));
    }

    #[test]
    fn overlaps_range_rejects_write_ending_before_line_start() {
        let line = ReservedLine::containing(0x1080);
        assert!(!line.overlaps_range(0x1000, 128));
    }

    #[test]
    fn overlaps_range_detects_write_ending_at_line_first_byte() {
        let line = ReservedLine::containing(0x1080);
        assert!(line.overlaps_range(0x1000, 129));
    }

    #[test]
    fn new_is_empty() {
        let t = ReservationTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn insert_new_entry_returns_none() {
        let mut t = ReservationTable::new();
        let prior = t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        assert!(prior.is_none());
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn insert_replace_returns_prior() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        let prior = t.insert_or_replace(unit(1), ReservedLine::containing(0x2000));
        assert_eq!(prior, Some(ReservedLine::containing(0x1000)));
        assert_eq!(t.get(unit(1)), Some(ReservedLine::containing(0x2000)));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn remove_present_returns_prior() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        let prior = t.remove_if_present(unit(1));
        assert_eq!(prior, Some(ReservedLine::containing(0x1000)));
        assert!(t.is_empty());
    }

    #[test]
    fn remove_absent_is_noop() {
        let mut t = ReservationTable::new();
        let prior = t.remove_if_present(unit(42));
        assert!(prior.is_none());
        assert!(t.is_empty());
    }

    #[test]
    fn get_missing_is_none() {
        let t = ReservationTable::new();
        assert!(t.get(unit(0)).is_none());
    }

    #[test]
    fn is_held_by_tracks_entries() {
        let mut t = ReservationTable::new();
        assert!(!t.is_held_by(unit(1)));
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        assert!(t.is_held_by(unit(1)));
        t.remove_if_present(unit(1));
        assert!(!t.is_held_by(unit(1)));
    }

    #[test]
    fn iter_is_in_unit_id_order() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(7), ReservedLine::containing(0x7000));
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        t.insert_or_replace(unit(3), ReservedLine::containing(0x3000));
        let ids: Vec<u64> = t.iter().map(|(u, _)| u.raw()).collect();
        assert_eq!(ids, vec![1, 3, 7]);
    }

    #[test]
    fn clear_covering_drops_overlapping_entries() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        t.insert_or_replace(unit(2), ReservedLine::containing(0x1080));
        t.insert_or_replace(unit(3), ReservedLine::containing(0x2000));
        let dropped = t.clear_covering(0x1040, 4);
        assert_eq!(dropped, 1);
        assert!(!t.is_held_by(unit(1)));
        assert!(t.is_held_by(unit(2)));
        assert!(t.is_held_by(unit(3)));
    }

    #[test]
    fn clear_covering_drops_all_entries_on_same_line() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        t.insert_or_replace(unit(2), ReservedLine::containing(0x1000));
        let dropped = t.clear_covering(0x1000, 4);
        assert_eq!(dropped, 2);
        assert!(t.is_empty());
    }

    #[test]
    fn clear_covering_zero_len_is_noop() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        let dropped = t.clear_covering(0x1000, 0);
        assert_eq!(dropped, 0);
        assert!(t.is_held_by(unit(1)));
    }

    #[test]
    fn clear_covering_empty_table_is_noop() {
        let mut t = ReservationTable::new();
        let dropped = t.clear_covering(0x1000, 128);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn clear_covering_spanning_write_clears_all_covered_lines() {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        t.insert_or_replace(unit(2), ReservedLine::containing(0x1080));
        t.insert_or_replace(unit(3), ReservedLine::containing(0x1100));
        t.insert_or_replace(unit(4), ReservedLine::containing(0x2000));
        let dropped = t.clear_covering(0x1000, 384);
        assert_eq!(dropped, 3);
        assert!(t.is_held_by(unit(4)));
        assert!(!t.is_held_by(unit(1)));
        assert!(!t.is_held_by(unit(2)));
        assert!(!t.is_held_by(unit(3)));
    }

    #[test]
    fn clear_covering_write_ending_at_next_line_start_does_not_touch_it() {
        // Boundary pin: a 256-byte write at 0x1000 covers bytes
        // [0x1000, 0x10FF], so it touches lines 0x1000 and 0x1080 but
        // NOT 0x1100.
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        t.insert_or_replace(unit(2), ReservedLine::containing(0x1080));
        t.insert_or_replace(unit(3), ReservedLine::containing(0x1100));
        let dropped = t.clear_covering(0x1000, 256);
        assert_eq!(dropped, 2);
        assert!(!t.is_held_by(unit(1)));
        assert!(!t.is_held_by(unit(2)));
        assert!(t.is_held_by(unit(3)));
    }

    #[test]
    fn state_hash_empty_is_stable() {
        let a = ReservationTable::new();
        let b = ReservationTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_content() {
        let mut a = ReservationTable::new();
        let b = ReservationTable::new();
        a.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_round_trips_after_clear() {
        let mut t = ReservationTable::new();
        let h0 = t.state_hash();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        assert_ne!(t.state_hash(), h0);
        t.remove_if_present(unit(1));
        assert_eq!(t.state_hash(), h0);
    }

    #[test]
    fn state_hash_distinguishes_line_addresses() {
        let mut a = ReservationTable::new();
        a.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        let mut b = ReservationTable::new();
        b.insert_or_replace(unit(1), ReservedLine::containing(0x2000));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_unit_ids() {
        let mut a = ReservationTable::new();
        a.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        let mut b = ReservationTable::new();
        b.insert_or_replace(unit(2), ReservedLine::containing(0x1000));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_insensitive_to_insertion_order() {
        let mut a = ReservationTable::new();
        a.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        a.insert_or_replace(unit(2), ReservedLine::containing(0x2000));
        a.insert_or_replace(unit(3), ReservedLine::containing(0x3000));

        let mut b = ReservationTable::new();
        b.insert_or_replace(unit(3), ReservedLine::containing(0x3000));
        b.insert_or_replace(unit(1), ReservedLine::containing(0x1000));
        b.insert_or_replace(unit(2), ReservedLine::containing(0x2000));

        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn pseudo_random_workload_is_deterministic() {
        fn run() -> (ReservationTable, u64) {
            let mut t = ReservationTable::new();
            let mut rng: u64 = 0xDEADBEEF_CAFEBABE;
            for _ in 0..256 {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let op = (rng >> 16) & 0x3;
                let uid = unit(rng & 0x7);
                let addr = (rng >> 24) & 0x0000_FFFF_FFFF_FF80;
                match op {
                    0 => {
                        t.insert_or_replace(uid, ReservedLine::containing(addr));
                    }
                    1 => {
                        t.remove_if_present(uid);
                    }
                    2 => {
                        t.clear_covering(addr, 4);
                    }
                    _ => {
                        t.clear_covering(addr & !0xFF, 256);
                    }
                }
            }
            let h = t.state_hash();
            (t, h)
        }

        let (a, ah) = run();
        let (b, bh) = run();
        assert_eq!(ah, bh);
        let a_entries: Vec<_> = a.iter().collect();
        let b_entries: Vec<_> = b.iter().collect();
        assert_eq!(a_entries, b_entries);
    }
}
