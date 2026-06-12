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
#[path = "tests/event_flag_tests.rs"]
mod tests;
