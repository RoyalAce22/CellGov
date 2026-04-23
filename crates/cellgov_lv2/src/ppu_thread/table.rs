//! Table of PPU threads owned by the LV2 host.

use super::block_reason::block_reason_payload;
use super::id::{PpuThreadId, PpuThreadIdAllocator};
use super::thread::{AddJoinWaiter, PpuThread, PpuThreadAttrs, PpuThreadState};
use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// Table of PPU threads owned by the LV2 host. Lookup by
/// `PpuThreadId` (guest-facing) or `UnitId` (runtime).
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

    /// Insert the primary thread. Must be called exactly once
    /// at host construction, before any `create` call.
    ///
    /// # Panics
    /// - If a primary thread has already been inserted.
    /// - If `create` has already run.
    /// - If `unit_id` already maps to another thread (debug).
    pub fn insert_primary(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        // These two are `assert!` rather than `debug_assert!`
        // because they guard a single-shot construction
        // invariant: the host calls this exactly once at boot,
        // so a release violation is a catastrophic wiring bug
        // that must not silently leave the host half-initialised.
        // The unit-id check below is `debug_assert!` because it
        // covers repeated per-thread bookkeeping where the
        // release-mode cost would add up.
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

    /// Create a new child thread and record its attributes.
    /// Returns the allocated id, or `None` if the id space is
    /// exhausted.
    ///
    /// # Panics (debug)
    /// If `unit_id` already maps to another thread.
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

    /// Mark a thread finished with the given exit value.
    /// Returns the list of joiners; the caller must transition
    /// those units back to `Runnable` and clear their block
    /// state.
    ///
    /// Returns an empty list if the thread does not exist or
    /// is already terminal.
    ///
    /// # Panics (debug)
    /// If called on a thread already `Finished` or `Detached`.
    pub fn mark_finished(&mut self, id: PpuThreadId, exit_value: u64) -> Vec<PpuThreadId> {
        let Some(thread) = self.threads.get_mut(&id) else {
            return Vec::new();
        };
        let already_terminal = matches!(
            thread.state,
            PpuThreadState::Finished | PpuThreadState::Detached
        );
        debug_assert!(
            !already_terminal,
            "mark_finished on {id:?} which is already {:?}",
            thread.state,
        );
        // Release guard matching the debug_assert: a second
        // call must not overwrite exit_value or drop Detached.
        // Joiners were drained on the first call.
        if already_terminal {
            return Vec::new();
        }
        thread.state = PpuThreadState::Finished;
        thread.exit_value = Some(exit_value);
        std::mem::take(&mut thread.join_waiters)
    }

    /// Destructively take the list of joiners parked on `id`
    /// without changing thread state. For non-destructive
    /// access use `get(id)?.join_waiters.as_slice()`.
    pub fn take_join_waiters(&mut self, id: PpuThreadId) -> Vec<PpuThreadId> {
        match self.threads.get_mut(&id) {
            Some(t) => std::mem::take(&mut t.join_waiters),
            None => Vec::new(),
        }
    }

    /// Append a waiter to the target's join list. The return
    /// variant names the exact outcome so callers can route
    /// each case to the right errno: see [`AddJoinWaiter`].
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

    /// Mark a thread `Detached`. Detached Finished threads are
    /// garbage-collected without a join. Returns `true` if the
    /// target exists.
    pub fn detach(&mut self, id: PpuThreadId) -> bool {
        match self.threads.get_mut(&id) {
            Some(t) => {
                t.state = PpuThreadState::Detached;
                true
            }
            None => false,
        }
    }

    /// Number of threads (including Finished / Detached that
    /// have not been purged).
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Iterate all thread ids in deterministic BTreeMap order.
    pub fn iter_ids(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.threads.keys().copied()
    }

    /// FNV-1a of the table for determinism checking. Folds id,
    /// unit_id, state, block reason (via
    /// [`super::GuestBlockReason::stable_tag`]), attrs, exit
    /// value, and the join-waiter list (length-prefixed to avoid
    /// boundary collisions) in BTreeMap order.
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
mod tests {
    use super::*;
    use crate::ppu_thread::{EventFlagWaitMode, GuestBlockReason};

    fn dummy_attrs() -> PpuThreadAttrs {
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        }
    }

    #[test]
    fn new_table_is_empty() {
        let t = PpuThreadTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn insert_primary_records_unit_mapping() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        assert_eq!(t.len(), 1);
        let p = t.get(PpuThreadId::PRIMARY).unwrap();
        assert_eq!(p.id, PpuThreadId::PRIMARY);
        assert_eq!(p.unit_id, UnitId::new(1));
        assert_eq!(p.state, PpuThreadState::Runnable);
        assert_eq!(
            t.thread_id_for_unit(UnitId::new(1)),
            Some(PpuThreadId::PRIMARY),
        );
    }

    #[test]
    #[should_panic(expected = "primary thread already inserted")]
    fn double_primary_insert_panics() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        t.insert_primary(UnitId::new(2), dummy_attrs());
    }

    #[test]
    #[should_panic(expected = "insert_primary called after create")]
    fn insert_primary_after_create_panics() {
        let mut t = PpuThreadTable::new();
        t.create(UnitId::new(1), dummy_attrs()).unwrap();
        t.insert_primary(UnitId::new(2), dummy_attrs());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already mapped to another thread")]
    fn create_with_duplicate_unit_id_panics_in_debug() {
        let mut t = PpuThreadTable::new();
        t.create(UnitId::new(1), dummy_attrs()).unwrap();
        t.create(UnitId::new(1), dummy_attrs());
    }

    #[test]
    fn create_allocates_above_primary() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        let c1 = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let c2 = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        assert_eq!(c1.raw(), 0x0100_0001);
        assert_eq!(c2.raw(), 0x0100_0002);
        assert!(c1 > PpuThreadId::PRIMARY);
        assert!(c2 > c1);
    }

    #[test]
    fn create_records_unit_and_attrs() {
        let mut t = PpuThreadTable::new();
        let mut attrs = dummy_attrs();
        attrs.arg = 0xdead_beef;
        let id = t.create(UnitId::new(5), attrs.clone()).unwrap();
        let thread = t.get(id).unwrap();
        assert_eq!(thread.unit_id, UnitId::new(5));
        assert_eq!(thread.attrs.arg, 0xdead_beef);
        assert_eq!(thread.state, PpuThreadState::Runnable);
        assert!(thread.join_waiters.is_empty());
        assert!(thread.exit_value.is_none());
        assert_eq!(t.get_by_unit(UnitId::new(5)).unwrap().id, id);
    }

    #[test]
    fn get_by_unit_unknown_returns_none() {
        let t = PpuThreadTable::new();
        assert!(t.get_by_unit(UnitId::new(99)).is_none());
    }

    #[test]
    fn mark_finished_sets_state_and_exit_value() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let waiters = t.mark_finished(id, 0x42);
        assert!(waiters.is_empty());
        let thread = t.get(id).unwrap();
        assert_eq!(thread.state, PpuThreadState::Finished);
        assert_eq!(thread.exit_value, Some(0x42));
    }

    #[test]
    fn mark_finished_unknown_returns_empty() {
        let mut t = PpuThreadTable::new();
        let waiters = t.mark_finished(PpuThreadId::new(0x9999), 0);
        assert!(waiters.is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already Finished")]
    fn mark_finished_twice_panics_in_debug() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.mark_finished(id, 0);
        t.mark_finished(id, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already Detached")]
    fn mark_finished_after_detach_panics_in_debug() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.detach(id);
        t.mark_finished(id, 0);
    }

    #[test]
    fn add_join_waiter_and_mark_finished_drain_waiters() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let third = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        assert_eq!(
            t.add_join_waiter(child, PpuThreadId::PRIMARY),
            AddJoinWaiter::Parked,
        );
        assert_eq!(t.add_join_waiter(child, third), AddJoinWaiter::Parked);
        let waiters = t.mark_finished(child, 0);
        assert_eq!(waiters, vec![PpuThreadId::PRIMARY, third]);
        assert!(t.get(child).unwrap().join_waiters.is_empty());
    }

    #[test]
    fn add_join_waiter_unknown_target_is_rejected() {
        let mut t = PpuThreadTable::new();
        assert_eq!(
            t.add_join_waiter(PpuThreadId::new(0x9999), PpuThreadId::PRIMARY),
            AddJoinWaiter::UnknownTarget,
        );
    }

    #[test]
    fn add_join_waiter_self_join_is_rejected() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        assert_eq!(t.add_join_waiter(id, id), AddJoinWaiter::SelfJoin);
        assert!(t.get(id).unwrap().join_waiters.is_empty());
    }

    #[test]
    fn add_join_waiter_on_finished_target_is_rejected() {
        let mut t = PpuThreadTable::new();
        let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let waiter = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        t.mark_finished(target, 0);
        assert_eq!(
            t.add_join_waiter(target, waiter),
            AddJoinWaiter::TargetAlreadyFinished,
        );
        assert!(t.get(target).unwrap().join_waiters.is_empty());
    }

    #[test]
    fn add_join_waiter_on_detached_target_is_rejected() {
        let mut t = PpuThreadTable::new();
        let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let waiter = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        t.detach(target);
        assert_eq!(
            t.add_join_waiter(target, waiter),
            AddJoinWaiter::TargetDetached,
        );
        assert!(t.get(target).unwrap().join_waiters.is_empty());
    }

    #[test]
    fn take_join_waiters_without_state_change() {
        let mut t = PpuThreadTable::new();
        let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.add_join_waiter(child, PpuThreadId::PRIMARY);
        let waiters = t.take_join_waiters(child);
        assert_eq!(waiters, vec![PpuThreadId::PRIMARY]);
        assert_eq!(t.get(child).unwrap().state, PpuThreadState::Runnable);
    }

    #[test]
    fn detach_sets_state() {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        assert!(t.detach(id));
        assert_eq!(t.get(id).unwrap().state, PpuThreadState::Detached);
    }

    #[test]
    fn detach_unknown_returns_false() {
        let mut t = PpuThreadTable::new();
        assert!(!t.detach(PpuThreadId::new(0x9999)));
    }

    #[test]
    fn state_hash_distinguishes_every_guest_block_reason() {
        fn table_with_reason(reason: GuestBlockReason) -> u64 {
            let mut t = PpuThreadTable::new();
            let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
            t.get_mut(id).unwrap().state = PpuThreadState::Blocked(reason);
            t.state_hash()
        }
        let hashes = [
            table_with_reason(GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            }),
            table_with_reason(GuestBlockReason::WaitingOnLwMutex { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnMutex { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnSemaphore { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnEventQueue { id: 1 }),
            table_with_reason(GuestBlockReason::WaitingOnEventFlag {
                id: 1,
                mask: 0,
                mode: EventFlagWaitMode::AndNoClear,
            }),
            table_with_reason(GuestBlockReason::WaitingOnCond {
                cond_id: 1,
                mutex_id: 1,
            }),
        ];
        for (i, h_i) in hashes.iter().enumerate() {
            for (j, h_j) in hashes.iter().enumerate().skip(i + 1) {
                assert_ne!(h_i, h_j, "variants {i} and {j} hash-collided");
            }
        }
    }

    #[test]
    fn state_hash_distinguishes_event_flag_wait_modes() {
        fn hash_with_mode(mode: EventFlagWaitMode) -> u64 {
            let mut t = PpuThreadTable::new();
            let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
            t.get_mut(id).unwrap().state =
                PpuThreadState::Blocked(GuestBlockReason::WaitingOnEventFlag {
                    id: 1,
                    mask: 0xAA,
                    mode,
                });
            t.state_hash()
        }
        let a = hash_with_mode(EventFlagWaitMode::AndNoClear);
        let b = hash_with_mode(EventFlagWaitMode::AndClear);
        let c = hash_with_mode(EventFlagWaitMode::OrNoClear);
        let d = hash_with_mode(EventFlagWaitMode::OrClear);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_ne!(b, c);
        assert_ne!(b, d);
        assert_ne!(c, d);
    }

    #[test]
    fn state_hash_empty_table_is_stable() {
        let a = PpuThreadTable::new();
        let b = PpuThreadTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_when_thread_added() {
        let empty = PpuThreadTable::new();
        let mut populated = PpuThreadTable::new();
        populated.create(UnitId::new(1), dummy_attrs()).unwrap();
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_changes_on_finish() {
        let mut a = PpuThreadTable::new();
        let mut b = PpuThreadTable::new();
        let id_a = a.create(UnitId::new(1), dummy_attrs()).unwrap();
        let id_b = b.create(UnitId::new(1), dummy_attrs()).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        a.mark_finished(id_a, 42);
        assert_ne!(a.state_hash(), b.state_hash());
        b.mark_finished(id_b, 42);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_folds_tls_base() {
        // tls_base is host-chosen, so a bug picking a wrong
        // TLS placement would otherwise go undetected.
        let mut a = PpuThreadTable::new();
        let mut b = PpuThreadTable::new();
        let mut attrs_a = dummy_attrs();
        let mut attrs_b = dummy_attrs();
        attrs_a.tls_base = 0x0020_0000;
        attrs_b.tls_base = 0x0030_0000;
        a.create(UnitId::new(1), attrs_a).unwrap();
        b.create(UnitId::new(1), attrs_b).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_join_waiter_list_length_is_load_bearing() {
        // (A=[X,Y], B=[]) vs (A=[X], B=[Y]) collide without a
        // length prefix.
        let mut a = PpuThreadTable::new();
        let mut b = PpuThreadTable::new();
        a.insert_primary(UnitId::new(1), dummy_attrs());
        b.insert_primary(UnitId::new(1), dummy_attrs());
        let a_child1 = a.create(UnitId::new(2), dummy_attrs()).unwrap();
        let _a_child2 = a.create(UnitId::new(3), dummy_attrs()).unwrap();
        let b_child1 = b.create(UnitId::new(2), dummy_attrs()).unwrap();
        let b_child2 = b.create(UnitId::new(3), dummy_attrs()).unwrap();
        a.add_join_waiter(a_child1, PpuThreadId::new(0x42));
        a.add_join_waiter(a_child1, PpuThreadId::new(0x43));
        b.add_join_waiter(b_child1, PpuThreadId::new(0x42));
        b.add_join_waiter(b_child2, PpuThreadId::new(0x43));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn iter_ids_returns_deterministic_order() {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(1), dummy_attrs());
        t.create(UnitId::new(2), dummy_attrs()).unwrap();
        t.create(UnitId::new(3), dummy_attrs()).unwrap();
        let ids: Vec<_> = t.iter_ids().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], PpuThreadId::PRIMARY);
        assert_eq!(ids[1].raw(), 0x0100_0001);
        assert_eq!(ids[2].raw(), 0x0100_0002);
        let ids2: Vec<_> = t.iter_ids().collect();
        assert_eq!(ids, ids2);
    }
}
