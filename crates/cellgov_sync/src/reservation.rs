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
use std::collections::BTreeMap;

// [CBE-Handbook p:577 s:20.2] CBE reservation granule is 128 bytes = PPE cache line.
pub use cellgov_ps3_abi::hardware::RESERVATION_LINE_BYTES;

// `containing()`'s line-mask arithmetic only aligns correctly when
// the granule is a power of two; catch a future non-power-of-two
// value at compile time.
const _: () = assert!(
    RESERVATION_LINE_BYTES.is_power_of_two(),
    "line mask arithmetic requires power-of-two granule"
);

use cellgov_ps3_abi::hardware::CELL_EA_LIMIT;

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

impl core::fmt::Display for ReservedLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.addr)
    }
}

/// Committed atomic-reservation state, at most one entry per unit.
/// A second `insert_or_replace` for the same unit drops the prior
/// entry (a second reserve invalidates the first).
// [PPC-Book2 p:10 s:1.7.3.1] "another lwarx/ldarx clears the first reservation".
#[derive(Debug, Clone, Default)]
pub struct ReservationTable {
    /// `BTreeMap` keeps `state_hash` invariant under permutation by
    /// walking unit ids in order. `UnitId: Ord` is what makes that
    /// determinism hold; it is not an incidental derive.
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
        self.entries.retain(|unit, line| {
            if Some(*unit) == except {
                return true;
            }
            !line.overlaps_range(addr, len)
        });
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
#[path = "tests/reservation_tests.rs"]
mod tests;
