//! Atomic reservation table shared across PPU and SPU.
//!
//! Models PPU `lwarx`/`ldarx` + `stwcx`/`stdcx` and SPU `MFC_GETLLAR` +
//! `MFC_PUTLLC` over a 128-byte cache-line granule. The commit
//! pipeline clears every entry whose line overlaps a committed write
//! from a *different* unit; a conditional store succeeds only if the
//! unit's entry is still present at commit time. The table holds the
//! global half of the verdict, ANDed with the unit's local
//! reservation register.
// [PPC-Book2 p:10 s:1.7.3.1] PPU lwarx/stwcx reservation + granule semantics; "another processor" stores clear the reservation.
// [CBE-Handbook p:590 s:20.3] SPU getllar/putllc 128-byte lock-line atomics.
//!
//! Keys are canonical line addresses (low 7 bits zero). Callers pass
//! byte-granular addresses; the table canonicalizes on insert.

use cellgov_event::UnitId;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

// [CBE-Handbook p:577 s:20.2] CBE reservation granule is 128 bytes = PPE cache line.
pub use cellgov_ps3_abi::hw::ppu::RESERVATION_LINE_BYTES;

// `containing()`'s line-mask arithmetic only aligns correctly when
// the granule is a power of two; catch a future non-power-of-two
// value at compile time.
const _: () = assert!(
    RESERVATION_LINE_BYTES.is_power_of_two(),
    "line mask arithmetic requires power-of-two granule"
);

use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;

/// 128-byte-aligned guest address. Low 7 bits are always zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReservedLine {
    addr: u64,
}

impl ReservedLine {
    /// Canonical line containing `byte_addr`.
    ///
    /// # Panics
    ///
    /// Debug-asserts `byte_addr` is within the 42-bit Cell BE EA
    /// space; beyond that, the saturating arithmetic regime makes
    /// overlap checks unreliable.
    #[inline]
    pub const fn containing(byte_addr: u64) -> Self {
        debug_assert!(
            byte_addr <= CELL_EA_LIMIT,
            "byte_addr exceeds Cell BE 42-bit EA space"
        );
        Self {
            addr: byte_addr & !(RESERVATION_LINE_BYTES - 1),
        }
    }

    /// Canonical (128-byte-aligned) line address.
    #[inline]
    pub const fn addr(self) -> u64 {
        self.addr
    }

    /// Inclusive last byte of this line. Saturating arithmetic; the
    /// `containing` debug-assert keeps in-spec call sites away from
    /// the saturation regime.
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

/// Field 1 is the line address.
impl LaneValue for ReservedLine {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.addr);
    }
}

impl core::fmt::Display for ReservedLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.addr)
    }
}

/// Committed atomic-reservation state, at most one entry per unit.
/// A second `insert_or_replace` for the same unit drops the prior
/// entry (a second reserve invalidates the first).
// [PPC-Book2 p:10 s:1.7.3.1] "another lwarx/ldarx clears the first reservation".
#[derive(Debug, Clone)]
pub struct ReservationTable {
    /// Walks unit ids in order, so `iter` is invariant under insertion
    /// order. The unit is the lane object and the address space is the
    /// slot base.
    entries: LaneMap<UnitId, ReservedLine>,
}

impl Default for ReservationTable {
    fn default() -> Self {
        Self::in_space(0)
    }
}

impl ReservationTable {
    /// Construct an empty table for the boot address space.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an empty table for address space `space`.
    #[inline]
    pub fn in_space(space: u32) -> Self {
        Self {
            entries: LaneMap::new(source::RESERVATION, UnitId::raw)
                .with_slot_base(u64::from(space)),
        }
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
        self.entries.remove(unit)
    }

    /// Read `unit`'s entry without mutating the table.
    #[inline]
    pub fn get(&self, unit: UnitId) -> Option<ReservedLine> {
        self.entries.get(unit).copied()
    }

    /// Committed-state half of the conditional-store verdict; the
    /// local-reservation-register check lives on the unit.
    #[inline]
    pub fn is_held_by(&self, unit: UnitId) -> bool {
        self.entries.contains_key(unit)
    }

    /// Iterate held reservations in unit-id order.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, ReservedLine)> + '_ {
        self.entries.iter().map(|(u, l)| (u, *l))
    }

    /// Drop every entry whose line overlaps `[addr, addr + len)`,
    /// except `except`'s own entry. Returns the count dropped. O(n)
    /// over entries.
    ///
    /// `except = Some(writer)` matches the spec: a unit's own store
    /// does not clear its own reservation. `None` means either the
    /// emitter's entry was dropped before this call (commit-side
    /// `ConditionalStore` path) or the writer is not a unit
    /// (privileged / external snoop).
    // [PPC-Book2 p:10 s:1.7.3.1] "some other processor executes a Store" -- own-unit stores do not clear.
    // [CBE-Handbook p:589 s:20.3] MFC atomic unit clears reservation on cross-processor snoop of granule.
    pub fn clear_covering(&mut self, addr: u64, len: u64, except: Option<UnitId>) -> usize {
        if self.entries.is_empty() || len == 0 {
            return 0;
        }
        let before = self.entries.len();
        self.entries
            .retain(|unit, line| Some(unit) == except || !line.overlaps_range(addr, len));
        before - self.entries.len()
    }

    /// The table's partial of the sync-state sum: per held reservation a
    /// presence lane and the line address.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/reservation_tests.rs"]
mod tests;
