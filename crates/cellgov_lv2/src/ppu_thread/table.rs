//! Table of PPU threads owned by the LV2 host.

use super::block_reason::block_reason_payload;
use super::id::{PpuThreadId, PpuThreadIdAllocator};
use super::thread::{AddJoinWaiter, PpuThread, PpuThreadAttrs, PpuThreadState};
use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// Table of PPU threads; lookup by `PpuThreadId` (guest-facing)
/// or `UnitId` (runtime).
#[derive(Debug, Clone, Default)]
pub struct PpuThreadTable {
    allocator: PpuThreadIdAllocator,
    threads: BTreeMap<PpuThreadId, PpuThread>,
    unit_to_thread: BTreeMap<UnitId, PpuThreadId>,
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
            !self.threads.contains_key(&PpuThreadId::PRIMARY),
            "primary thread already inserted",
        );
        assert!(
            self.threads.is_empty(),
            "insert_primary called after create; table has {} non-primary entries",
            self.threads.len(),
        );
        debug_assert!(
            !self.unit_to_thread.contains_key(&unit_id),
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
            !self.unit_to_thread.contains_key(&unit_id),
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
    /// the calling (primary) thread (see RPCS3
    /// `_sys_prx_start_module(ppu_thread&, ...)`,
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_prx.cpp:515`).
    /// Aliasing the transient unit to the primary thread here
    /// gives sync-syscall dispatch sites a real PpuThreadId for
    /// the caller without weakening the strict lookup elsewhere.
    ///
    /// # Errors
    /// Returns `false` if `existing` is not a known thread or
    /// `unit_id` is already mapped (the caller is responsible
    /// for not double-aliasing).
    pub fn alias_unit(&mut self, unit_id: UnitId, existing: PpuThreadId) -> bool {
        if !self.threads.contains_key(&existing) {
            return false;
        }
        if self.unit_to_thread.contains_key(&unit_id) {
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
        self.unit_to_thread.remove(&unit_id).is_some()
    }

    /// Look up a thread by id.
    pub fn get(&self, id: PpuThreadId) -> Option<&PpuThread> {
        self.threads.get(&id)
    }

    /// Mutably look up a thread by id.
    pub fn get_mut(&mut self, id: PpuThreadId) -> Option<&mut PpuThread> {
        self.threads.get_mut(&id)
    }

    /// Look up a thread by its runtime unit id.
    pub fn get_by_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.unit_to_thread
            .get(&unit_id)
            .and_then(|id| self.threads.get(id))
    }

    /// Mutably look up a thread by its runtime unit id.
    pub fn get_by_unit_mut(&mut self, unit_id: UnitId) -> Option<&mut PpuThread> {
        let id = *self.unit_to_thread.get(&unit_id)?;
        self.threads.get_mut(&id)
    }

    /// Translate a runtime unit id to its guest thread id.
    pub fn thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.unit_to_thread.get(&unit_id).copied()
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
        let Some(thread) = self.threads.get_mut(&id) else {
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
    /// thread state; for read-only access use
    /// `get(id)?.join_waiters.as_slice()`.
    pub fn take_join_waiters(&mut self, id: PpuThreadId) -> Vec<PpuThreadId> {
        match self.threads.get_mut(&id) {
            Some(t) => std::mem::take(&mut t.join_waiters),
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
        let Some(t) = self.threads.get_mut(&target) else {
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
        match self.threads.get_mut(&id) {
            Some(t) => {
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

    /// Iterate all thread ids in BTreeMap order.
    pub fn iter_ids(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.threads.keys().copied()
    }

    /// Whether any thread other than `caller` is alive (not yet
    /// `Finished` / `Detached`).
    ///
    /// Used by finite-timeout wait dispatchers to decide between
    /// blocking and tripping `CELL_ETIMEDOUT`: if no peer thread can
    /// possibly post / set the awaited condition, blocking would
    /// deadlock the schedule, so the wait must time-trip immediately.
    pub fn has_other_alive_thread(&self, caller: PpuThreadId) -> bool {
        self.threads
            .iter()
            .any(|(id, t)| *id != caller && t.state.is_alive())
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

    /// FNV-1a fold for determinism checking.
    ///
    /// Walks entries in BTreeMap order and folds id, unit_id,
    /// state tag, block-reason tag + payload (via
    /// [`super::GuestBlockReason::stable_tag`]), attrs, exit
    /// value, and the join-waiter list length-prefixed so
    /// `([X,Y], [])` cannot collide with `([X], [Y])`.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, thread) in &self.threads {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&thread.unit_id.raw().to_le_bytes());
            let state_byte = match &thread.state {
                PpuThreadState::Runnable => 0u8,
                PpuThreadState::Blocked(_) => 1,
                PpuThreadState::Finished => 2,
                PpuThreadState::Detached => 3,
            };
            hasher.write(&[state_byte]);
            if let PpuThreadState::Blocked(reason) = &thread.state {
                hasher.write(&[reason.stable_tag()]);
                let payload = block_reason_payload(reason);
                hasher.write(&payload);
            }
            let a = &thread.attrs;
            hasher.write(&a.entry.to_le_bytes());
            hasher.write(&a.arg.to_le_bytes());
            hasher.write(&a.stack_base.to_le_bytes());
            hasher.write(&a.stack_size.to_le_bytes());
            hasher.write(&a.priority.to_le_bytes());
            hasher.write(&a.tls_base.to_le_bytes());
            if let Some(v) = thread.exit_value {
                hasher.write(&[1]);
                hasher.write(&v.to_le_bytes());
            } else {
                hasher.write(&[0]);
            }
            hasher.write(&(thread.join_waiters.len() as u64).to_le_bytes());
            for waiter in &thread.join_waiters {
                hasher.write(&waiter.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod tests;
