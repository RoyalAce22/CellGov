//! Event flag table.
//!
//! Each entry holds `bits: u64` plus a FIFO waiter list.
//! `set_and_wake` ORs in the new bits then walks the list waking
//! each waiter whose mask matches its mode. CLEAR-wakes mutate
//! `bits` mid-walk, so two waiters from a single call can
//! observe different patterns.

use crate::ppu_thread::{EventFlagWaitMode, PpuThreadId};
use std::collections::BTreeMap;

/// One parked waiter on an event flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlagWaiter {
    /// Parked thread.
    pub thread: PpuThreadId,
    /// Bit mask the waiter expects.
    pub mask: u64,
    /// AND/OR match policy and CLEAR/NO-CLEAR wake policy.
    pub mode: EventFlagWaitMode,
    /// Guest address to write the observed pattern on wake.
    pub result_ptr: u32,
}

/// One woken waiter's continuation, emitted in FIFO order by
/// `set_and_wake`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlagWake {
    /// Woken thread.
    pub thread: PpuThreadId,
    /// `bits` at the moment this waiter's predicate fired; may
    /// differ across waiters from one call when an earlier
    /// CLEAR-wake mutated `bits`.
    pub observed: u64,
    /// Guest address to write the observed pattern to.
    pub result_ptr: u32,
}

/// Outcome of a `try_wait` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagWait {
    /// Mask matched; `observed` is the pre-clear pattern.
    Matched {
        /// Pre-clear bit pattern.
        observed: u64,
    },
    /// Mask did not match.
    NoMatch,
}

/// Failure modes of [`EventFlagTable::create_with_id`].
///
/// `IdCollision` indicates an allocator bug; `debug_assert!`
/// fires. Release keeps the existing entry and returns `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagCreateError {
    /// An entry with this id was already present.
    IdCollision,
}

/// Failure modes of [`EventFlagTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagEnqueueError {
    /// No event flag with this id.
    UnknownId,
    /// Thread is already parked on this flag; dispatch-layer bug
    /// (fires `debug_assert!`).
    DuplicateWaiter,
}

/// A single event flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFlagEntry {
    bits: u64,
    init: u64,
    waiters: Vec<EventFlagWaiter>,
}

impl EventFlagEntry {
    fn new(init: u64) -> Self {
        Self {
            bits: init,
            init,
            waiters: Vec::new(),
        }
    }

    /// Current bit state.
    pub fn bits(&self) -> u64 {
        self.bits
    }

    /// Initial bit state captured at create time.
    pub fn init(&self) -> u64 {
        self.init
    }

    /// Read-only iterator over parked waiters in enqueue order.
    pub fn waiters(&self) -> &[EventFlagWaiter] {
        &self.waiters
    }
}

fn mask_matches(bits: u64, mask: u64, mode: EventFlagWaitMode) -> bool {
    match mode {
        EventFlagWaitMode::AndClear | EventFlagWaitMode::AndNoClear => (bits & mask) == mask,
        EventFlagWaitMode::OrClear | EventFlagWaitMode::OrNoClear => (bits & mask) != 0,
    }
}

fn should_clear(mode: EventFlagWaitMode) -> bool {
    matches!(
        mode,
        EventFlagWaitMode::AndClear | EventFlagWaitMode::OrClear
    )
}

/// Table of event flags.
#[derive(Debug, Clone, Default)]
pub struct EventFlagTable {
    entries: BTreeMap<u32, EventFlagEntry>,
}

impl EventFlagTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry. See [`EventFlagCreateError`].
    pub fn create_with_id(&mut self, id: u32, init: u64) -> Result<(), EventFlagCreateError> {
        if let Some(existing) = self.entries.get(&id) {
            debug_assert!(
                false,
                "event flag {:#x} already present (existing init={:#x} bits={:#x} waiters={}, new init={:#x})",
                id,
                existing.init,
                existing.bits,
                existing.waiters.len(),
                init,
            );
            return Err(EventFlagCreateError::IdCollision);
        }
        self.entries.insert(id, EventFlagEntry::new(init));
        Ok(())
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling
    /// (`debug_assert!` fires on violation). If bypassed in
    /// release, callers **must** drain `entry.waiters()` and wake
    /// each parked thread; skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<EventFlagEntry> {
        let entry = self.entries.remove(&id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "event flag {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&EventFlagEntry> {
        self.entries.get(&id)
    }

    /// Number of tracked event flags.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Try to wait without parking. Applies CLEAR on match.
    pub fn try_wait(
        &mut self,
        id: u32,
        mask: u64,
        mode: EventFlagWaitMode,
    ) -> Option<EventFlagWait> {
        let entry = self.entries.get_mut(&id)?;
        if mask_matches(entry.bits, mask, mode) {
            let observed = entry.bits;
            if should_clear(mode) {
                entry.bits &= !mask;
            }
            Some(EventFlagWait::Matched { observed })
        } else {
            Some(EventFlagWait::NoMatch)
        }
    }

    /// Park a waiter.
    ///
    /// Precondition: caller must have seen
    /// [`EventFlagWait::NoMatch`] from [`Self::try_wait`].
    /// Parking on an already-matching mask would strand the
    /// waiter until some future set mutates `bits`;
    /// `debug_assert!` catches it.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        thread: PpuThreadId,
        mask: u64,
        mode: EventFlagWaitMode,
        result_ptr: u32,
    ) -> Result<(), EventFlagEnqueueError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(EventFlagEnqueueError::UnknownId)?;
        debug_assert!(
            !mask_matches(entry.bits, mask, mode),
            "thread {:?} enqueued on event flag {:#x}: bits {:#x} already match mask {:#x} under {:?}",
            thread,
            id,
            entry.bits,
            mask,
            mode,
        );
        if entry.waiters.iter().any(|w| w.thread == thread) {
            debug_assert!(
                false,
                "duplicate enqueue of {:?} on event flag {:#x}",
                thread, id,
            );
            return Err(EventFlagEnqueueError::DuplicateWaiter);
        }
        entry.waiters.push(EventFlagWaiter {
            thread,
            mask,
            mode,
            result_ptr,
        });
        Ok(())
    }

    /// OR `bits_to_set` into the flag and wake every matching
    /// waiter in FIFO order; `None` if `id` is unknown.
    pub fn set_and_wake(&mut self, id: u32, bits_to_set: u64) -> Option<Vec<EventFlagWake>> {
        let entry = self.entries.get_mut(&id)?;
        entry.bits |= bits_to_set;
        let mut woken: Vec<EventFlagWake> = Vec::new();
        let mut i = 0;
        while i < entry.waiters.len() {
            let w = entry.waiters[i];
            if mask_matches(entry.bits, w.mask, w.mode) {
                let observed = entry.bits;
                if should_clear(w.mode) {
                    entry.bits &= !w.mask;
                }
                entry.waiters.remove(i);
                woken.push(EventFlagWake {
                    thread: w.thread,
                    observed,
                    result_ptr: w.result_ptr,
                });
                // remove(i) shifted the next waiter into slot i.
                continue;
            }
            i += 1;
        }
        Some(woken)
    }

    /// Drain every parked waiter without mutating `bits`. Each
    /// returned [`EventFlagWaiter`] is the full record so the
    /// caller can wake the thread with whatever return code they
    /// choose (typically `CELL_ECANCELED`). Returns `None` if `id`
    /// is unknown.
    pub fn cancel_waiters(&mut self, id: u32) -> Option<Vec<EventFlagWaiter>> {
        let entry = self.entries.get_mut(&id)?;
        Some(std::mem::take(&mut entry.waiters))
    }

    /// `entry.bits &= mask` -- the LV2 `sys_event_flag_clear` keeps
    /// bits **inside** `mask` and drops the rest, matching RPCS3's
    /// `pattern.atomic_op(|p| p &= bitptn)`. The argument name
    /// `bits_to_clear` is a misnomer kept for backwards
    /// compatibility with callers, but the semantics are mask-and,
    /// not bit-clear. Returns `false` if `id` is unknown.
    pub fn clear_bits(&mut self, id: u32, bits_to_clear: u64) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.bits &= bits_to_clear;
        true
    }

    /// FNV-1a digest of the table's state.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            hasher.write(&entry.bits.to_le_bytes());
            hasher.write(&entry.init.to_le_bytes());
            hasher.write(&(entry.waiters.len() as u64).to_le_bytes());
            for w in &entry.waiters {
                hasher.write(&w.thread.raw().to_le_bytes());
                hasher.write(&w.mask.to_le_bytes());
                hasher.write(&[w.mode.stable_tag()]);
                hasher.write(&w.result_ptr.to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(raw: u64) -> PpuThreadId {
        PpuThreadId::new(raw)
    }

    #[test]
    fn fresh_table_is_empty() {
        let t = EventFlagTable::new();
        assert!(t.is_empty());
    }

    #[test]
    fn try_wait_and_mode_requires_all_bits() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b1010).unwrap();
        assert_eq!(
            t.try_wait(1, 0b1000, EventFlagWaitMode::AndNoClear),
            Some(EventFlagWait::Matched { observed: 0b1010 }),
        );
        assert_eq!(t.lookup(1).unwrap().bits(), 0b1010);
        assert_eq!(
            t.try_wait(1, 0b0101, EventFlagWaitMode::AndNoClear),
            Some(EventFlagWait::NoMatch),
        );
    }

    #[test]
    fn try_wait_or_mode_requires_any_bit() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b0010).unwrap();
        assert_eq!(
            t.try_wait(1, 0b1010, EventFlagWaitMode::OrNoClear),
            Some(EventFlagWait::Matched { observed: 0b0010 }),
        );
        assert_eq!(
            t.try_wait(1, 0b1100, EventFlagWaitMode::OrNoClear),
            Some(EventFlagWait::NoMatch),
        );
    }

    #[test]
    fn try_wait_clear_mode_clears_matched_bits() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b1111).unwrap();
        assert_eq!(
            t.try_wait(1, 0b1010, EventFlagWaitMode::AndClear),
            Some(EventFlagWait::Matched { observed: 0b1111 }),
        );
        assert_eq!(t.lookup(1).unwrap().bits(), 0b0101);
    }

    #[test]
    fn set_with_no_waiters_just_ors_bits() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b0001).unwrap();
        let woken = t.set_and_wake(1, 0b1010).unwrap();
        assert!(woken.is_empty());
        assert_eq!(t.lookup(1).unwrap().bits(), 0b1011);
    }

    #[test]
    fn set_wakes_matching_waiter_in_fifo_order() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0010,
            EventFlagWaitMode::AndNoClear,
            0x2020,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0003),
            0b1000,
            EventFlagWaitMode::AndNoClear,
            0x2040,
        )
        .unwrap();
        let woken = t.set_and_wake(1, 0b0011).unwrap();
        assert_eq!(
            woken,
            vec![
                EventFlagWake {
                    thread: tid(0x0100_0001),
                    observed: 0b0011,
                    result_ptr: 0x2000,
                },
                EventFlagWake {
                    thread: tid(0x0100_0002),
                    observed: 0b0011,
                    result_ptr: 0x2020,
                },
            ],
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
        assert_eq!(t.lookup(1).unwrap().waiters()[0].thread, tid(0x0100_0003));
    }

    #[test]
    fn set_with_clear_waiter_clears_bits_before_next_waiter_check() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndClear,
            0x2000,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2020,
        )
        .unwrap();
        let woken = t.set_and_wake(1, 0b0001).unwrap();
        assert_eq!(
            woken,
            vec![EventFlagWake {
                thread: tid(0x0100_0001),
                observed: 0b0001,
                result_ptr: 0x2000,
            }],
        );
        assert_eq!(t.lookup(1).unwrap().bits(), 0);
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    fn set_and_wake_clear_then_noclear_shows_different_observed() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndClear,
            0x2000,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0010,
            EventFlagWaitMode::AndNoClear,
            0x2020,
        )
        .unwrap();
        let woken = t.set_and_wake(1, 0b0011).unwrap();
        assert_eq!(
            woken,
            vec![
                EventFlagWake {
                    thread: tid(0x0100_0001),
                    observed: 0b0011,
                    result_ptr: 0x2000,
                },
                EventFlagWake {
                    thread: tid(0x0100_0002),
                    observed: 0b0010,
                    result_ptr: 0x2020,
                },
            ],
        );
        assert_eq!(t.lookup(1).unwrap().bits(), 0b0010);
    }

    #[test]
    fn clear_bits_masks_without_waking() {
        // sys_event_flag_clear is mask-and: bits in `mask` survive,
        // bits outside `mask` are dropped. Matches RPCS3
        // (`pattern &= bitptn`). Old value `0b0111` masked by
        // `0b0101` -> `0b0101`.
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b0111).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1000,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        assert!(t.clear_bits(1, 0b0101));
        assert_eq!(t.lookup(1).unwrap().bits(), 0b0101);
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    fn unknown_id_returns_none() {
        let mut t = EventFlagTable::new();
        assert!(t.try_wait(99, 0b1, EventFlagWaitMode::AndNoClear).is_none());
        assert!(t.set_and_wake(99, 0b1).is_none());
        assert!(!t.clear_bits(99, 0b1));
        assert_eq!(
            t.enqueue_waiter(99, tid(1), 0b1, EventFlagWaitMode::AndNoClear, 0x100),
            Err(EventFlagEnqueueError::UnknownId),
        );
    }

    #[test]
    fn set_wakes_each_waiter_with_its_own_result_ptr() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x1000,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0010,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0003),
            0b0100,
            EventFlagWaitMode::AndNoClear,
            0x3000,
        )
        .unwrap();
        let woken = t.set_and_wake(1, 0b0111).unwrap();
        assert_eq!(
            woken,
            vec![
                EventFlagWake {
                    thread: tid(0x0100_0001),
                    observed: 0b0111,
                    result_ptr: 0x1000,
                },
                EventFlagWake {
                    thread: tid(0x0100_0002),
                    observed: 0b0111,
                    result_ptr: 0x2000,
                },
                EventFlagWake {
                    thread: tid(0x0100_0003),
                    observed: 0b0111,
                    result_ptr: 0x3000,
                },
            ],
        );
    }

    #[test]
    fn state_hash_distinguishes_waiter_mode() {
        let mut a = EventFlagTable::new();
        let mut b = EventFlagTable::new();
        a.create_with_id(1, 0).unwrap();
        b.create_with_id(1, 0).unwrap();
        a.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndClear,
            0x2000,
        )
        .unwrap();
        b.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already present")]
    fn create_with_id_panics_on_collision_in_debug() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0xAA).unwrap();
        let _ = t.create_with_id(1, 0xBB);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "duplicate enqueue")]
    fn duplicate_enqueue_panics_in_debug() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        let _ = t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already match")]
    fn enqueue_on_already_matching_bits_panics_in_debug() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b1111).unwrap();
        let _ = t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "destroyed with")]
    fn destroy_with_parked_waiters_panics_in_debug() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        let _ = t.destroy(1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn create_with_id_returns_collision_err_in_release() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0xAA).unwrap();
        assert_eq!(
            t.create_with_id(1, 0xBB),
            Err(EventFlagCreateError::IdCollision),
        );
        assert_eq!(t.lookup(1).unwrap().init(), 0xAA);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn duplicate_enqueue_returns_err_in_release() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        assert_eq!(
            t.enqueue_waiter(
                1,
                tid(0x0100_0001),
                0b1,
                EventFlagWaitMode::AndNoClear,
                0x2000,
            ),
            Err(EventFlagEnqueueError::DuplicateWaiter),
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn enqueue_on_already_matching_bits_still_parks_in_release() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b1111).unwrap();
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        )
        .unwrap();
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
        let woken = t.set_and_wake(1, 0).unwrap();
        assert_eq!(woken.len(), 1);
    }
}
