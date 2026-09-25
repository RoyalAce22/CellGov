//! Event flag table.
//!
//! CLEAR-wakes mutate `bits` mid-walk of the waiter list, so two
//! waiters woken by one `set_and_wake` call can observe different
//! patterns.

use crate::ppu_thread::{EventFlagWaitMode, PpuThreadId};
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

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
    /// `bits` at the moment this waiter's predicate fired.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventFlagCreateError {
    /// An entry with this id was already present.
    #[error("event_flag create: {0}")]
    IdCollision(#[source] super::IdCollision),
}

/// Failure modes of [`EventFlagTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventFlagEnqueueError {
    /// No event flag with this id.
    #[error("event_flag enqueue: unknown id")]
    UnknownId,
    /// Thread is already parked on this flag; dispatch-layer bug
    /// (fires `debug_assert!`).
    #[error("event_flag enqueue: duplicate waiter")]
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
#[derive(Debug, Clone)]
pub struct EventFlagTable {
    entries: LaneMap<u32, EventFlagEntry>,
}

impl Default for EventFlagTable {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::EVENT_FLAG, u64::from),
        }
    }
}

impl LaneValue for EventFlagEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bits);
        lanes.lane(2, 0, self.init);
        lanes.lane(3, 0, self.waiters.len() as u64);
        for (slot, w) in self.waiters.iter().enumerate() {
            let slot = slot as u64;
            lanes.lane(4, slot, w.thread.raw());
            lanes.lane(5, slot, w.mask);
            lanes.lane(6, slot, u64::from(w.mode.stable_tag()) + 1);
            lanes.lane(7, slot, u64::from(w.result_ptr));
        }
    }
}

impl EventFlagTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry. See [`EventFlagCreateError`].
    pub fn create_with_id(&mut self, id: u32, init: u64) -> Result<(), EventFlagCreateError> {
        if let Some(existing) = self.entries.get(id) {
            debug_assert!(
                false,
                "event flag {:#x} already present (existing init={:#x} bits={:#x} waiters={}, new init={:#x})",
                id,
                existing.init,
                existing.bits,
                existing.waiters.len(),
                init,
            );
            return Err(EventFlagCreateError::IdCollision(super::IdCollision { id }));
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
        let entry = self.entries.remove(id)?;
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
        self.entries.get(id)
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
        let mut entry = self.entries.get_mut(id)?;
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
        let mut entry = self
            .entries
            .get_mut(id)
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
        let mut entry = self.entries.get_mut(id)?;
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

    /// Remove `thread`'s waiter record without mutating `bits`;
    /// `None` if the id is unknown or the thread is not parked.
    /// Timeout-expiry cancel, unlike the all-or-nothing
    /// [`Self::cancel_waiters`].
    pub fn remove_waiter(&mut self, id: u32, thread: PpuThreadId) -> Option<EventFlagWaiter> {
        let mut entry = self.entries.get_mut(id)?;
        let pos = entry.waiters.iter().position(|w| w.thread == thread)?;
        Some(entry.waiters.remove(pos))
    }

    /// Remove every waiter in `threads` from every flag without
    /// mutating `bits` or writing result pointers, preserving the
    /// order of survivors; returns `(id, thread)` pairs in table
    /// order. Process-exit purge.
    #[must_use = "the purged pairs are the only witness that these wakes were cancelled"]
    pub fn purge_waiters_of(
        &mut self,
        threads: &std::collections::BTreeSet<PpuThreadId>,
    ) -> Vec<(u32, PpuThreadId)> {
        let mut removed = Vec::new();
        self.entries.for_each_mut(|id, entry| {
            entry.waiters.retain(|w| {
                if threads.contains(&w.thread) {
                    removed.push((id, w.thread));
                    false
                } else {
                    true
                }
            });
        });
        removed
    }

    /// Drain every parked waiter without mutating `bits`; `None`
    /// if `id` is unknown.
    ///
    /// Caller must wake each returned waiter, typically with
    /// `CELL_ECANCELED`.
    pub fn cancel_waiters(&mut self, id: u32) -> Option<Vec<EventFlagWaiter>> {
        let mut entry = self.entries.get_mut(id)?;
        Some(std::mem::take(&mut entry.waiters))
    }

    /// `entry.bits &= mask` -- `sys_event_flag_clear` ANDs the flag
    /// value with the caller's pattern, so a 1 bit keeps and a 0 bit
    /// clears.
    ///
    /// The hardware trace settles that reading: in
    /// `tests/ps3autotests/tests/lv2/sys_event_flag` a flag holding
    /// `0x1f`, cleared with `0xaaaaaaaaaaaaaaaa`, reads back `0x0a`.
    ///
    /// Returns `false` if `id` is unknown.
    pub fn clear_bits(&mut self, id: u32, mask: u64) -> bool {
        let Some(mut entry) = self.entries.get_mut(id) else {
            return false;
        };
        entry.bits &= mask;
        true
    }

    /// The table's partial of the sync-state sum.
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/event_flag_tests.rs"]
mod tests;
