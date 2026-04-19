//! Event flag table.
//!
//! Owns the state for `sys_event_flag_create` / `_destroy` /
//! `_wait` / `_set` / `_clear` / `_trywait`. Each entry holds
//! `bits: u64` (the current flag state) and a FIFO waiter list
//! where each waiter carries its own `wait_mask` and
//! `wait_mode`.
//!
//! Wait mode encodes two orthogonal bits per the PS3 ABI:
//!   * Match policy: AND (all bits in mask must be set) vs OR
//!     (any bit in mask set).
//!   * Wake policy: CLEAR (matched bits are cleared on wake) vs
//!     NO-CLEAR (bits preserved).
//!
//! Set semantics: OR the new bits into `bits`, then walk the
//! waiter list in FIFO order. Any waiter whose mask now matches
//! per its mode is woken. If the waiter's mode is CLEAR, the
//! matched bits are cleared before the next waiter is evaluated
//! (so a CLEAR-wake can prevent a subsequent NO-CLEAR waiter
//! from firing on the same set). Multiple waiters can wake from
//! a single set call when they match on disjoint bit sets.

use crate::ppu_thread::{EventFlagWaitMode, PpuThreadId};
use std::collections::BTreeMap;

/// One parked waiter on an event flag.
///
/// Each waiter carries its own `result_ptr` (the continuation
/// pointer supplied at wait time) so `set_and_wake` can emit a
/// complete `PendingResponse::EventFlagWake` per matched waiter
/// without looking up the parked response in the runtime's
/// syscall-response table. Co-locating continuation state with
/// the waiter entry is the shared pattern across primitives
/// whose wake delivers data to guest memory (see event queue's
/// `EventQueueWaiter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlagWaiter {
    /// Thread parked on this event flag.
    pub thread: PpuThreadId,
    /// Bit mask the waiter expects.
    pub mask: u64,
    /// AND/OR match policy and CLEAR/NO-CLEAR wake policy.
    pub mode: EventFlagWaitMode,
    /// Guest address to write the observed bit pattern on wake.
    pub result_ptr: u32,
}

/// One woken waiter's continuation: the thread to wake, the
/// bit pattern it observes, and the `result_ptr` the wait
/// handler recorded at park time. Emitted in FIFO order by
/// `set_and_wake`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlagWake {
    /// Thread woken.
    pub thread: PpuThreadId,
    /// Observed bit pattern at wake time.
    pub observed: u64,
    /// Guest address to write the observed pattern to.
    pub result_ptr: u32,
}

/// Outcome of a `try_wait` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagWait {
    /// The mask matched; caller returns CELL_OK. `observed` is
    /// the bit pattern the caller sees (after CLEAR if
    /// applicable).
    Matched {
        /// Bits observed before any CLEAR was applied.
        observed: u64,
    },
    /// Mask did not match -- caller should park (for `_wait`) or
    /// return EBUSY (for `_trywait`).
    NoMatch,
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

    /// Insert a fresh entry with the given id and initial bit
    /// state. Returns `false` if `id` is already present.
    pub fn create_with_id(&mut self, id: u32, init: u64) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }
        self.entries.insert(id, EventFlagEntry::new(init));
        true
    }

    /// Destroy an event flag. Returns the removed entry or `None`.
    pub fn destroy(&mut self, id: u32) -> Option<EventFlagEntry> {
        self.entries.remove(&id)
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

    /// Try to wait without parking. If the mask matches per mode,
    /// apply the CLEAR if applicable and return the observed
    /// pattern. Otherwise return `NoMatch` with state unchanged.
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

    /// Park `waiter` on the flag's waiter list. Returns `false`
    /// if `id` is unknown or the thread is already parked.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        thread: PpuThreadId,
        mask: u64,
        mode: EventFlagWaitMode,
        result_ptr: u32,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.waiters.iter().any(|w| w.thread == thread) {
            return false;
        }
        entry.waiters.push(EventFlagWaiter {
            thread,
            mask,
            mode,
            result_ptr,
        });
        true
    }

    /// Set (OR in) bits on the flag and walk the waiter list
    /// in FIFO order waking any waiter whose mask now matches.
    /// Returns one `EventFlagWake` per matched waiter in FIFO
    /// order. Returns `None` if `id` is unknown.
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
                // Do not advance `i` -- next waiter shifted into
                // this slot. Continue walking because a CLEAR-wake
                // may have prevented a later waiter from matching,
                // but a NO-CLEAR wake may still match subsequent
                // waiters on the same bits.
                continue;
            }
            i += 1;
        }
        Some(woken)
    }

    /// Clear (AND-NOT) bits on the flag. Does not wake anyone.
    /// Returns `false` if `id` is unknown.
    pub fn clear_bits(&mut self, id: u32, bits_to_clear: u64) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.bits &= !bits_to_clear;
        true
    }

    /// FNV-1a digest.
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
                hasher.write(&[w.mode as u8]);
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
        t.create_with_id(1, 0b1010);
        assert_eq!(
            t.try_wait(1, 0b1000, EventFlagWaitMode::AndNoClear),
            Some(EventFlagWait::Matched { observed: 0b1010 }),
        );
        // bits unchanged (no clear).
        assert_eq!(t.lookup(1).unwrap().bits(), 0b1010);
        assert_eq!(
            t.try_wait(1, 0b0101, EventFlagWaitMode::AndNoClear),
            Some(EventFlagWait::NoMatch),
        );
    }

    #[test]
    fn try_wait_or_mode_requires_any_bit() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b0010);
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
        t.create_with_id(1, 0b1111);
        assert_eq!(
            t.try_wait(1, 0b1010, EventFlagWaitMode::AndClear),
            Some(EventFlagWait::Matched { observed: 0b1111 }),
        );
        assert_eq!(t.lookup(1).unwrap().bits(), 0b0101);
    }

    #[test]
    fn set_with_no_waiters_just_ors_bits() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b0001);
        let woken = t.set_and_wake(1, 0b1010).unwrap();
        assert!(woken.is_empty());
        assert_eq!(t.lookup(1).unwrap().bits(), 0b1011);
    }

    #[test]
    fn set_wakes_matching_waiter_in_fifo_order() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0);
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0010,
            EventFlagWaitMode::AndNoClear,
            0x2020,
        );
        t.enqueue_waiter(
            1,
            tid(0x0100_0003),
            0b1000,
            EventFlagWaitMode::AndNoClear,
            0x2040,
        );
        // Set bits 0b0011 -- waiters 1 and 2 match; waiter 3
        // still waits (bit 0b1000 not set).
        let woken = t.set_and_wake(1, 0b0011).unwrap();
        // FIFO wake order; each carries its own result_ptr.
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
        // w3 still parked.
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
        assert_eq!(t.lookup(1).unwrap().waiters()[0].thread, tid(0x0100_0003));
    }

    #[test]
    fn set_with_clear_waiter_clears_bits_before_next_waiter_check() {
        // w1 has CLEAR on mask 0b0001; w2 has NO-CLEAR on mask
        // 0b0001. Set 0b0001: w1 matches, clears bit 0. w2 then
        // checks -- bit already cleared -- no match. w2 stays
        // parked.
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0);
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndClear,
            0x2000,
        );
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x2020,
        );
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
    fn clear_bits_removes_without_waking() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0b1111);
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1000,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
        assert!(t.clear_bits(1, 0b0101));
        assert_eq!(t.lookup(1).unwrap().bits(), 0b1010);
        // Waiter still parked even though its mask still matches;
        // clear never wakes anyone.
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    fn duplicate_enqueue_rejected() {
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0);
        assert!(t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000
        ));
        assert!(!t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000
        ));
    }

    #[test]
    fn unknown_id_returns_none() {
        let mut t = EventFlagTable::new();
        assert!(t.try_wait(99, 0b1, EventFlagWaitMode::AndNoClear).is_none());
        assert!(t.set_and_wake(99, 0b1).is_none());
        assert!(!t.clear_bits(99, 0b1));
    }

    #[test]
    fn set_wakes_each_waiter_with_its_own_result_ptr() {
        // Three waiters on the same flag, each with a distinct
        // result_ptr. A single set that matches all three must
        // return each waiter's own continuation pointer, not a
        // shared placeholder. The old sentinel-merge design
        // accidentally worked because the parked responses were
        // populated at wait time; the new per-waiter storage
        // makes the per-continuation tracking explicit.
        let mut t = EventFlagTable::new();
        t.create_with_id(1, 0);
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b0001,
            EventFlagWaitMode::AndNoClear,
            0x1000,
        );
        t.enqueue_waiter(
            1,
            tid(0x0100_0002),
            0b0010,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
        t.enqueue_waiter(
            1,
            tid(0x0100_0003),
            0b0100,
            EventFlagWaitMode::AndNoClear,
            0x3000,
        );
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
        a.create_with_id(1, 0);
        b.create_with_id(1, 0);
        a.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndClear,
            0x2000,
        );
        b.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        );
        assert_ne!(a.state_hash(), b.state_hash());
    }
}
