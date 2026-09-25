//! Table of PPU threads owned by the LV2 host.

use super::block_reason::block_reason_payload;
use super::id::{PpuThreadId, PpuThreadIdAllocator};
use super::thread::{AddJoinWaiter, PpuThread, PpuThreadAttrs, PpuThreadState};
use cellgov_event::UnitId;
use cellgov_mem::lanes::{self, source, LaneEntryMut, LaneMap, LaneValue, ObjectLanes};

impl LaneValue for PpuThread {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.unit_id.raw());
        let (state, reason) = match &self.state {
            PpuThreadState::Runnable => (1, None),
            PpuThreadState::Blocked(reason) => (2, Some(reason)),
            PpuThreadState::Finished => (3, None),
            PpuThreadState::Detached => (4, None),
        };
        lanes.lane(2, 0, state);
        if let Some(reason) = reason {
            lanes.lane(3, 0, u64::from(reason.stable_tag()));
            let payload = block_reason_payload(reason);
            for (i, chunk) in payload.chunks_exact(8).enumerate() {
                let mut word = [0u8; 8];
                word.copy_from_slice(chunk);
                lanes.lane(4, i as u64, u64::from_le_bytes(word));
            }
        }
        let a = &self.attrs;
        lanes.lane(5, 0, a.entry);
        lanes.lane(6, 0, a.arg);
        lanes.lane(7, 0, u64::from(a.stack_base));
        lanes.lane(8, 0, u64::from(a.stack_size));
        lanes.lane(9, 0, u64::from(a.priority));
        lanes.lane(10, 0, u64::from(a.tls_base));
        lanes.lane(11, 0, u64::from(self.exit_value.is_some()));
        lanes.lane(12, 0, self.exit_value.unwrap_or(0));
        lanes.lane(13, 0, self.join_waiters.len() as u64);
        for (slot, waiter) in self.join_waiters.iter().enumerate() {
            lanes.lane(14, slot as u64, waiter.raw());
        }
    }
}

impl LaneValue for PpuThreadId {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.raw());
    }
}

/// Table of PPU threads; lookup by `PpuThreadId` (guest-facing)
/// or `UnitId` (runtime).
#[derive(Debug, Clone)]
pub struct PpuThreadTable {
    allocator: PpuThreadIdAllocator,
    threads: LaneMap<PpuThreadId, PpuThread>,
    unit_to_thread: LaneMap<UnitId, PpuThreadId>,
}

impl Default for PpuThreadTable {
    fn default() -> Self {
        Self {
            allocator: PpuThreadIdAllocator::new(),
            threads: LaneMap::new(source::PPU_THREAD, PpuThreadId::raw),
            unit_to_thread: LaneMap::new(source::PPU_THREAD_UNIT, UnitId::raw),
        }
    }
}

impl PpuThreadTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert the primary thread; must be called exactly once at
    /// host construction, before any `create`.
    ///
    /// # Panics
    /// - If a primary thread has already been inserted.
    /// - If `create` has already run.
    /// - Debug-only if `unit_id` already maps to another thread.
    pub fn insert_primary(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        assert!(
            !self.threads.contains_key(PpuThreadId::PRIMARY),
            "primary thread already inserted",
        );
        assert!(
            self.threads.is_empty(),
            "insert_primary called after create; table has {} non-primary entries",
            self.threads.len(),
        );
        debug_assert!(
            !self.unit_to_thread.contains_key(unit_id),
            "insert_primary: UnitId {unit_id:?} already mapped to another thread",
        );
        let thread = PpuThread {
            id: PpuThreadId::PRIMARY,
            unit_id,
            state: PpuThreadState::Runnable,
            attrs,
            exit_value: None,
            join_waiters: Vec::new(),
        };
        self.threads.insert(PpuThreadId::PRIMARY, thread);
        self.unit_to_thread.insert(unit_id, PpuThreadId::PRIMARY);
    }

    /// Create a child thread and record its attributes; `None`
    /// if the id space is exhausted.
    ///
    /// # Panics
    /// Debug-only if `unit_id` already maps to another thread.
    pub fn create(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) -> Option<PpuThreadId> {
        debug_assert!(
            !self.unit_to_thread.contains_key(unit_id),
            "create: UnitId {unit_id:?} already mapped to another thread",
        );
        let id = self.allocator.allocate()?;
        let thread = PpuThread {
            id,
            unit_id,
            state: PpuThreadState::Runnable,
            attrs,
            exit_value: None,
            join_waiters: Vec::new(),
        };
        self.threads.insert(id, thread);
        self.unit_to_thread.insert(unit_id, id);
        Some(id)
    }

    /// Insert an explicit `unit_id -> existing_thread_id` alias.
    ///
    /// Cross-module contract: the bootstrap loop runs each PRX's
    /// module_start on a transient PPU unit that has no thread
    /// record of its own; real LV2 attributes those syscalls to
    /// the calling (primary) thread. liblv2's `sys_prx_start_module`
    /// takes the module lock, asks the kernel for the module's start
    /// entry, and calls that entry through `bctrl` on the calling
    /// thread. It creates no thread, so every syscall the entry
    /// issues comes from the caller.
    ///
    /// # Errors
    /// Returns `false` if `existing` is not a known thread or
    /// `unit_id` is already mapped (the caller is responsible
    /// for not double-aliasing).
    pub fn alias_unit(&mut self, unit_id: UnitId, existing: PpuThreadId) -> bool {
        if !self.threads.contains_key(existing) {
            return false;
        }
        if self.unit_to_thread.contains_key(unit_id) {
            return false;
        }
        self.unit_to_thread.insert(unit_id, existing);
        true
    }

    /// Remove an alias previously installed via [`Self::alias_unit`].
    ///
    /// Has no effect on the underlying `PpuThread`. The bootstrap
    /// loop drops aliases after the title primary takes over so
    /// post-boot lookups against the retired transient `UnitId`s
    /// fall through to the strict ESRCH path.
    pub fn drop_alias(&mut self, unit_id: UnitId) -> bool {
        self.unit_to_thread.remove(unit_id).is_some()
    }

    /// Look up a thread by id.
    pub fn get(&self, id: PpuThreadId) -> Option<&PpuThread> {
        self.threads.get(id)
    }

    /// Mutably look up a thread by id.
    pub fn get_mut(&mut self, id: PpuThreadId) -> Option<LaneEntryMut<'_, PpuThreadId, PpuThread>> {
        self.threads.get_mut(id)
    }

    /// Look up a thread by its runtime unit id.
    pub fn get_by_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.unit_to_thread
            .get(unit_id)
            .and_then(|id| self.threads.get(*id))
    }

    /// Mutably look up a thread by its runtime unit id.
    pub fn get_by_unit_mut(
        &mut self,
        unit_id: UnitId,
    ) -> Option<LaneEntryMut<'_, PpuThreadId, PpuThread>> {
        let id = *self.unit_to_thread.get(unit_id)?;
        self.threads.get_mut(id)
    }

    /// Translate a runtime unit id to its guest thread id.
    pub fn thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.unit_to_thread.get(unit_id).copied()
    }

    /// Remove every thread in `threads` from every thread's
    /// join-waiter list; returns `(target, waiter)` pairs in
    /// ascending target order, and within a target in the order the
    /// waiters were parked.
    ///
    /// Process-exit purge: a purged joiner must never be handed the
    /// target's exit value on a later `mark_finished`.
    ///
    /// # Cross-module contract
    ///
    /// Join-side record only: nothing is woken and no
    /// `SyscallResponseTable` entry is cleared, so every purged
    /// waiter stays parked. The process-exit caller depends on that
    /// -- its threads are being finished, not resumed -- so any other
    /// caller owes each returned waiter a wake of its own.
    pub fn purge_join_waiters_of(
        &mut self,
        threads: &std::collections::BTreeSet<PpuThreadId>,
    ) -> Vec<(PpuThreadId, PpuThreadId)> {
        let mut removed = Vec::new();
        self.threads.for_each_mut(|target, thread| {
            thread.join_waiters.retain(|&waiter| {
                if threads.contains(&waiter) {
                    removed.push((target, waiter));
                    false
                } else {
                    true
                }
            });
        });
        removed
    }

    /// Mark a thread finished and return its drained joiners.
    ///
    /// Caller must transition every returned unit back to
    /// `Runnable` and clear its block state; leaking the list
    /// leaks parked threads.
    ///
    /// Empty result if the thread does not exist or is already
    /// terminal.
    ///
    /// # Panics
    /// Debug-only if called on a thread already `Finished` or
    /// `Detached`.
    pub fn mark_finished(&mut self, id: PpuThreadId, exit_value: u64) -> Vec<PpuThreadId> {
        let Some(mut thread) = self.threads.get_mut(id) else {
            return Vec::new();
        };
        debug_assert!(
            thread.state.is_alive(),
            "mark_finished on {id:?} which is already {:?}",
            thread.state,
        );
        // Release guard: a second call must not overwrite
        // exit_value or drop Detached. Joiners drained on the
        // first call.
        if !thread.state.is_alive() {
            return Vec::new();
        }
        thread.state = PpuThreadState::Finished;
        thread.exit_value = Some(exit_value);
        std::mem::take(&mut thread.join_waiters)
    }

    /// Destructively take the joiner list without changing
    /// thread state.
    pub fn take_join_waiters(&mut self, id: PpuThreadId) -> Vec<PpuThreadId> {
        match self.threads.get_mut(id) {
            Some(mut t) => std::mem::take(&mut t.join_waiters),
            None => Vec::new(),
        }
    }

    /// Append a waiter to the target's join list; the returned
    /// [`AddJoinWaiter`] variant names the exact outcome so
    /// callers can route each case to the right errno.
    pub fn add_join_waiter(&mut self, target: PpuThreadId, waiter: PpuThreadId) -> AddJoinWaiter {
        if target == waiter {
            return AddJoinWaiter::SelfJoin;
        }
        let Some(mut t) = self.threads.get_mut(target) else {
            return AddJoinWaiter::UnknownTarget;
        };
        match t.state {
            PpuThreadState::Finished => AddJoinWaiter::TargetAlreadyFinished,
            PpuThreadState::Detached => AddJoinWaiter::TargetDetached,
            PpuThreadState::Runnable | PpuThreadState::Blocked(_) => {
                t.join_waiters.push(waiter);
                AddJoinWaiter::Parked
            }
        }
    }

    /// Mark a thread `Detached` so it garbage-collects on
    /// finish without a join; `true` if the target exists.
    pub fn detach(&mut self, id: PpuThreadId) -> bool {
        match self.threads.get_mut(id) {
            Some(mut t) => {
                t.state = PpuThreadState::Detached;
                true
            }
            None => false,
        }
    }

    /// Number of threads, including unpurged Finished / Detached.
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Iterate all thread ids in ascending order.
    pub fn iter_ids(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.threads.keys()
    }

    /// Whether any thread whose low-32-bit id matches `raw_low32` is
    /// still alive. The user-space owner field of `sys_lwmutex_t` only
    /// stores the low 32 bits of the kernel `PpuThreadId`, so the HLE
    /// lwmutex fast-path cannot distinguish between two threads whose
    /// ids share a low-32-bit prefix; in practice every allocated
    /// thread id we hand out has a unique low-32 chunk, so this match
    /// is exact.
    ///
    /// Returns `true` if there is no thread with that id (the owner
    /// field carries a stale id from a thread that was never seeded
    /// here, which we treat as alive to keep the contention path).
    pub fn is_owner_alive(&self, raw_low32: u32) -> bool {
        let Some((_, thread)) = self
            .threads
            .iter()
            .find(|(id, _)| (id.raw() as u32) == raw_low32)
        else {
            return true;
        };
        thread.state.is_alive()
    }

    /// The table's partial of the sync-state sum: the threads, the unit
    /// bindings and the id allocator's cursor.
    pub fn sync_partial(&self) -> u128 {
        self.threads
            .partial()
            .wrapping_add(self.unit_to_thread.partial())
            .wrapping_add(self.allocator_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.threads
            .partial_from_scratch()
            .wrapping_add(self.unit_to_thread.partial_from_scratch())
            .wrapping_add(self.allocator_term())
    }

    /// The id allocator's term.
    fn allocator_term(&self) -> u128 {
        lanes::value_term(source::PPU_THREAD_IDS, 0, &self.allocator)
    }
}

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod tests;
