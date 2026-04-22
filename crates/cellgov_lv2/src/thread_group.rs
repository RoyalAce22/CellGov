//! Managed SPU thread group table.
//!
//! Owns group lifecycle from create/initialize/start through finish.
//! Group ids are monotonic u32 tokens starting at 1 (0 reserved).

use crate::dispatch::{SpuImageHandle, SpuInitState};
use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// Cap on slots per group; the `group_id * 256 + slot` thread-id
/// encoding aliases adjacent groups' ranges above this.
pub const MAX_SLOTS_PER_GROUP: u32 = 256;

/// A single slot within a thread group.
#[derive(Debug, Clone)]
pub struct ThreadSlot {
    /// Image handle from `sys_spu_image_open`.
    pub image_handle: SpuImageHandle,
    /// Captured at initialize time: the PPU may reuse the same
    /// stack variable across initialize calls.
    pub args: [u64; 4],
    /// `None` until `sys_spu_thread_group_start` resolves the image.
    pub init: Option<SpuInitState>,
}

/// Lifecycle state of a thread group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Created but not yet started.
    Created,
    /// Started; SPUs are running.
    Running,
    /// All SPUs finished.
    Finished,
}

/// A managed SPU thread group.
#[derive(Debug, Clone)]
pub struct ThreadGroup {
    /// Group id.
    pub id: u32,
    /// Expected SPU slot count, declared at create time.
    pub num_threads: u32,
    /// Populated slots, keyed by slot index.
    pub slots: BTreeMap<u32, ThreadSlot>,
    /// Current lifecycle state.
    pub state: GroupState,
    /// Drives the terminal transition: the group flips to
    /// [`GroupState::Finished`] when this reaches 0.
    pub remaining_unfinished: u32,
}

/// Failure modes of [`ThreadGroupTable::initialize_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializeThreadError {
    /// No group with this id.
    UnknownGroup,
    /// Slots can only be populated while the group is
    /// [`GroupState::Created`].
    GroupAlreadyStarted {
        /// Current group state.
        state: GroupState,
    },
    /// Slot index is `>= num_threads` declared at create time.
    SlotOutOfBounds {
        /// The rejected slot index.
        slot: u32,
        /// The group's declared slot count.
        num_threads: u32,
    },
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    SlotOutOfRange,
    /// The slot already has an entry.
    SlotAlreadyInitialized,
}

/// Failure modes of [`ThreadGroupTable::record_spu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordSpuError {
    /// No group with this id.
    UnknownGroup,
    /// Registering against a finished group would inflate the
    /// counter without triggering the terminal transition.
    GroupAlreadyFinished,
    /// `unit_id` is already registered in some group, or was
    /// registered in one that has since finished.
    DuplicateUnit,
    /// `(group_id, slot)` already maps to a different unit.
    ThreadIdCollision,
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    SlotOutOfRange,
    /// `group_id * 256 + slot` overflows `u32`.
    ThreadIdOverflow,
}

/// Failure modes of [`ThreadGroupTable::notify_spu_finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifySpuFinishedError {
    /// `unit_id` was never registered as an SPU. Benign for
    /// non-SPU units; callers iterating every unit can ignore.
    UnknownUnit,
    /// `unit_id` was registered and its notify already fired.
    /// Distinct from [`Self::UnknownUnit`] so a double-notify
    /// against a live unit does not masquerade as a
    /// non-SPU-unit call.
    AlreadyFinished {
        /// Group this unit belonged to.
        group_id: u32,
    },
    /// Owning group is not in [`GroupState::Running`]
    /// (finish-before-start).
    GroupNotRunning {
        /// Current group state.
        state: GroupState,
    },
    /// `remaining_unfinished` was already 0.
    CounterUnderflow,
}

/// Table of managed SPU thread groups.
#[derive(Debug, Clone, Default)]
pub struct ThreadGroupTable {
    groups: BTreeMap<u32, ThreadGroup>,
    /// Live SPU registrations. `notify_spu_finished` moves entries
    /// out of here and into `finished_units`; that move is what
    /// lets a double-notify surface as
    /// [`NotifySpuFinishedError::AlreadyFinished`].
    unit_to_group: BTreeMap<UnitId, u32>,
    /// Units whose notify has already fired. Separate from
    /// `unit_to_group` so `UnknownUnit` and `AlreadyFinished`
    /// stay distinguishable.
    finished_units: BTreeMap<UnitId, u32>,
    /// Synthetic thread_id (`group_id * 256 + slot`) to runtime UnitId.
    thread_id_to_unit: BTreeMap<u32, UnitId>,
    next_id: u32,
}

impl ThreadGroupTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self {
            groups: BTreeMap::new(),
            unit_to_group: BTreeMap::new(),
            finished_units: BTreeMap::new(),
            thread_id_to_unit: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Create a group with `num_threads` slots.
    /// Returns `None` if the u32 id space is exhausted.
    pub fn create(&mut self, num_threads: u32) -> Option<u32> {
        let id = self.next_id;
        if id == 0 {
            return None;
        }
        self.next_id = id.wrapping_add(1);
        let group = ThreadGroup {
            id,
            num_threads,
            slots: BTreeMap::new(),
            state: GroupState::Created,
            remaining_unfinished: 0,
        };
        self.groups.insert(id, group);
        Some(id)
    }

    /// Populate a slot in a group (from `sys_spu_thread_initialize`).
    ///
    /// # Errors
    /// See [`InitializeThreadError`].
    pub fn initialize_thread(
        &mut self,
        group_id: u32,
        slot: u32,
        image_handle: SpuImageHandle,
        args: [u64; 4],
    ) -> Result<(), InitializeThreadError> {
        let group = self
            .groups
            .get_mut(&group_id)
            .ok_or(InitializeThreadError::UnknownGroup)?;
        if group.state != GroupState::Created {
            return Err(InitializeThreadError::GroupAlreadyStarted { state: group.state });
        }
        if slot >= MAX_SLOTS_PER_GROUP {
            return Err(InitializeThreadError::SlotOutOfRange);
        }
        if slot >= group.num_threads {
            return Err(InitializeThreadError::SlotOutOfBounds {
                slot,
                num_threads: group.num_threads,
            });
        }
        if group.slots.contains_key(&slot) {
            return Err(InitializeThreadError::SlotAlreadyInitialized);
        }
        group.slots.insert(
            slot,
            ThreadSlot {
                image_handle,
                args,
                init: None,
            },
        );
        Ok(())
    }

    /// Look up a group by id.
    pub fn get(&self, group_id: u32) -> Option<&ThreadGroup> {
        self.groups.get(&group_id)
    }

    /// Mutably look up a group by id.
    pub fn get_mut(&mut self, group_id: u32) -> Option<&mut ThreadGroup> {
        self.groups.get_mut(&group_id)
    }

    /// Number of groups.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Register `unit_id` as an SPU in `group_id` at `slot`.
    ///
    /// # Errors
    /// See [`RecordSpuError`]. No state is mutated on failure.
    pub fn record_spu(
        &mut self,
        unit_id: UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), RecordSpuError> {
        // All validation runs before any insert so an Err leaves
        // the table untouched.
        {
            let group = self
                .groups
                .get(&group_id)
                .ok_or(RecordSpuError::UnknownGroup)?;
            if group.state == GroupState::Finished {
                return Err(RecordSpuError::GroupAlreadyFinished);
            }
        }
        if slot >= MAX_SLOTS_PER_GROUP {
            return Err(RecordSpuError::SlotOutOfRange);
        }
        let thread_id = group_id
            .checked_mul(MAX_SLOTS_PER_GROUP)
            .and_then(|x| x.checked_add(slot))
            .ok_or(RecordSpuError::ThreadIdOverflow)?;
        if self.unit_to_group.contains_key(&unit_id) || self.finished_units.contains_key(&unit_id) {
            return Err(RecordSpuError::DuplicateUnit);
        }
        if self.thread_id_to_unit.contains_key(&thread_id) {
            return Err(RecordSpuError::ThreadIdCollision);
        }
        self.unit_to_group.insert(unit_id, group_id);
        self.thread_id_to_unit.insert(thread_id, unit_id);
        let group = self
            .groups
            .get_mut(&group_id)
            .expect("group existence checked above and table is &mut");
        group.remaining_unfinished = group
            .remaining_unfinished
            .checked_add(1)
            .expect("remaining_unfinished overflows u32: more than 2^32 SPUs in one group");
        Ok(())
    }

    /// Look up the runtime UnitId for a synthetic thread_id.
    pub fn unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        self.thread_id_to_unit.get(&thread_id).copied()
    }

    /// Thread-id lookup filtered to [`GroupState::Running`].
    ///
    /// Mailbox writes and other state-sensitive paths must use
    /// this so writes to not-yet-started or already-finished
    /// threads surface as `ESRCH` rather than queuing in a dead
    /// mailbox.
    pub fn running_unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        let unit_id = self.unit_for_thread(thread_id)?;
        let group_id = self.unit_to_group.get(&unit_id)?;
        let group = self.groups.get(group_id)?;
        if group.state == GroupState::Running {
            Some(unit_id)
        } else {
            None
        }
    }

    /// Notify that the SPU `unit_id` has finished.
    ///
    /// `Ok(Some(group_id))` means this notify drove the group to
    /// `Finished`; `Ok(None)` means the group still has live SPUs.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]. Only `UnknownUnit` is
    /// benign; the others indicate dispatch-layer state
    /// corruption.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: UnitId,
    ) -> Result<Option<u32>, NotifySpuFinishedError> {
        if let Some(&group_id) = self.finished_units.get(&unit_id) {
            return Err(NotifySpuFinishedError::AlreadyFinished { group_id });
        }
        let group_id = *self
            .unit_to_group
            .get(&unit_id)
            .ok_or(NotifySpuFinishedError::UnknownUnit)?;
        // Existence is guaranteed: `record_spu` checks the group
        // before inserting into `unit_to_group`, and the table
        // exposes no group-destroy path.
        let group = self
            .groups
            .get_mut(&group_id)
            .expect("unit_to_group references a nonexistent group");
        if group.state != GroupState::Running {
            return Err(NotifySpuFinishedError::GroupNotRunning { state: group.state });
        }
        if group.remaining_unfinished == 0 {
            return Err(NotifySpuFinishedError::CounterUnderflow);
        }
        group.remaining_unfinished -= 1;
        let terminal = group.remaining_unfinished == 0;
        if terminal {
            group.state = GroupState::Finished;
        }
        // Move to `finished_units` so a second notify hits
        // AlreadyFinished instead of re-decrementing.
        self.unit_to_group.remove(&unit_id);
        self.finished_units.insert(unit_id, group_id);
        if terminal {
            Ok(Some(group_id))
        } else {
            Ok(None)
        }
    }

    /// FNV-1a hash of the table for determinism checking.
    ///
    /// Folds `unit_to_group`, `finished_units`, and
    /// `thread_id_to_unit` independently even though the latter
    /// two overlap: each is separately mutable, so a desync is
    /// only catchable if each is hashed on its own.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, group) in &self.groups {
            hasher.write(&id.to_le_bytes());
            hasher.write(&group.num_threads.to_le_bytes());
            hasher.write(&group.remaining_unfinished.to_le_bytes());
            let state_byte = match group.state {
                GroupState::Created => 0u8,
                GroupState::Running => 1,
                GroupState::Finished => 2,
            };
            hasher.write(&[state_byte]);
            hasher.write(&(group.slots.len() as u64).to_le_bytes());
            for (slot_idx, slot) in &group.slots {
                hasher.write(&slot_idx.to_le_bytes());
                hasher.write(&slot.image_handle.raw().to_le_bytes());
                for a in &slot.args {
                    hasher.write(&a.to_le_bytes());
                }
                match &slot.init {
                    None => hasher.write(&[0u8]),
                    Some(init) => {
                        hasher.write(&[1u8]);
                        hasher.write(&(init.ls_bytes.len() as u64).to_le_bytes());
                        hasher.write(&init.ls_bytes);
                        hasher.write(&init.entry_pc.to_le_bytes());
                        hasher.write(&init.stack_ptr.to_le_bytes());
                        for a in &init.args {
                            hasher.write(&a.to_le_bytes());
                        }
                        hasher.write(&init.group_id.to_le_bytes());
                    }
                }
            }
        }
        hasher.write(&(self.unit_to_group.len() as u64).to_le_bytes());
        for (uid, gid) in &self.unit_to_group {
            hasher.write(&uid.raw().to_le_bytes());
            hasher.write(&gid.to_le_bytes());
        }
        hasher.write(&(self.finished_units.len() as u64).to_le_bytes());
        for (uid, gid) in &self.finished_units {
            hasher.write(&uid.raw().to_le_bytes());
            hasher.write(&gid.to_le_bytes());
        }
        hasher.write(&(self.thread_id_to_unit.len() as u64).to_le_bytes());
        for (tid, uid) in &self.thread_id_to_unit {
            hasher.write(&tid.to_le_bytes());
            hasher.write(&uid.raw().to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(raw: u32) -> SpuImageHandle {
        SpuImageHandle::new(raw).unwrap()
    }

    #[test]
    fn new_table_is_empty() {
        let t = ThreadGroupTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn create_allocates_ids_starting_at_1() {
        let mut t = ThreadGroupTable::new();
        let g1 = t.create(2).unwrap();
        let g2 = t.create(4).unwrap();
        assert_eq!(g1, 1);
        assert_eq!(g2, 2);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn create_records_num_threads() {
        let mut t = ThreadGroupTable::new();
        let id = t.create(3).unwrap();
        let group = t.get(id).unwrap();
        assert_eq!(group.num_threads, 3);
        assert_eq!(group.state, GroupState::Created);
        assert!(group.slots.is_empty());
    }

    #[test]
    fn initialize_thread_populates_slot() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.initialize_thread(gid, 0, img(1), [0x2000, 0, 0, 0])
            .unwrap();
        let group = t.get(gid).unwrap();
        assert_eq!(group.slots.len(), 1);
        assert_eq!(group.slots[&0].image_handle, img(1));
        assert_eq!(group.slots[&0].args[0], 0x2000);
    }

    #[test]
    fn initialize_thread_unknown_group_returns_err() {
        let mut t = ThreadGroupTable::new();
        assert_eq!(
            t.initialize_thread(99, 0, img(1), [0; 4]),
            Err(InitializeThreadError::UnknownGroup),
        );
    }

    #[test]
    fn initialize_thread_rejects_slot_out_of_bounds() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        assert_eq!(
            t.initialize_thread(gid, 5, img(1), [0; 4]),
            Err(InitializeThreadError::SlotOutOfBounds {
                slot: 5,
                num_threads: 2,
            }),
        );
        assert!(t.get(gid).unwrap().slots.is_empty());
    }

    #[test]
    fn initialize_thread_rejects_slot_at_encoding_limit() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(MAX_SLOTS_PER_GROUP + 10).unwrap();
        assert_eq!(
            t.initialize_thread(gid, MAX_SLOTS_PER_GROUP, img(1), [0; 4]),
            Err(InitializeThreadError::SlotOutOfRange),
        );
    }

    #[test]
    fn initialize_thread_rejects_duplicate_slot() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.initialize_thread(gid, 0, img(1), [1, 0, 0, 0]).unwrap();
        assert_eq!(
            t.initialize_thread(gid, 0, img(2), [2, 0, 0, 0]),
            Err(InitializeThreadError::SlotAlreadyInitialized),
        );
        // Previous values preserved.
        let slot = &t.get(gid).unwrap().slots[&0];
        assert_eq!(slot.image_handle, img(1));
        assert_eq!(slot.args[0], 1);
    }

    #[test]
    fn initialize_thread_rejects_started_group() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        assert_eq!(
            t.initialize_thread(gid, 0, img(1), [0; 4]),
            Err(InitializeThreadError::GroupAlreadyStarted {
                state: GroupState::Running,
            }),
        );
    }

    #[test]
    fn state_hash_is_deterministic() {
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(2).unwrap();
        let gb = b.create(2).unwrap();
        a.initialize_thread(ga, 0, img(1), [0; 4]).unwrap();
        b.initialize_thread(gb, 0, img(1), [0; 4]).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_content() {
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        a.create(2).unwrap();
        b.create(3).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_empty_vs_populated_differ() {
        let empty = ThreadGroupTable::new();
        let mut populated = ThreadGroupTable::new();
        populated.create(1).unwrap();
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_folds_slot_args() {
        // Two tables differing only in args must hash differently.
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(1).unwrap();
        let gb = b.create(1).unwrap();
        a.initialize_thread(ga, 0, img(1), [0x1111, 0, 0, 0])
            .unwrap();
        b.initialize_thread(gb, 0, img(1), [0x2222, 0, 0, 0])
            .unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_folds_slot_init() {
        let init_a = SpuInitState {
            ls_bytes: vec![0xAA; 16],
            entry_pc: 0,
            stack_ptr: 0,
            args: [0; 4],
            group_id: 1,
        };
        let init_b = SpuInitState {
            ls_bytes: vec![0xBB; 16],
            ..init_a.clone()
        };
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(1).unwrap();
        let gb = b.create(1).unwrap();
        a.initialize_thread(ga, 0, img(1), [0; 4]).unwrap();
        b.initialize_thread(gb, 0, img(1), [0; 4]).unwrap();
        a.get_mut(ga).unwrap().slots.get_mut(&0).unwrap().init = Some(init_a);
        b.get_mut(gb).unwrap().slots.get_mut(&0).unwrap().init = Some(init_b);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_folds_thread_id_to_unit() {
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(1).unwrap();
        let gb = b.create(1).unwrap();
        a.get_mut(ga).unwrap().state = GroupState::Running;
        b.get_mut(gb).unwrap().state = GroupState::Running;
        a.record_spu(UnitId::new(10), ga, 0).unwrap();
        b.record_spu(UnitId::new(11), gb, 0).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn group_id_zero_never_allocated() {
        let mut t = ThreadGroupTable::new();
        for _ in 0..5 {
            let id = t.create(1).unwrap();
            assert_ne!(id, 0);
        }
    }

    #[test]
    fn create_returns_none_when_id_space_exhausted() {
        let mut t = ThreadGroupTable::new();
        t.next_id = u32::MAX;
        let last = t.create(1).unwrap();
        assert_eq!(last, u32::MAX);
        assert!(t.create(1).is_none());
    }

    #[test]
    fn running_unit_for_thread_returns_unit_when_group_running() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        let uid = UnitId::new(42);
        t.record_spu(uid, gid, 0).unwrap();
        let thread_id = gid * MAX_SLOTS_PER_GROUP;
        assert_eq!(t.running_unit_for_thread(thread_id), Some(uid));
    }

    #[test]
    fn running_unit_for_thread_returns_none_for_created_group() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        // Group left in Created state (not started).
        let uid = UnitId::new(42);
        t.record_spu(uid, gid, 0).unwrap();
        let thread_id = gid * MAX_SLOTS_PER_GROUP;
        assert_eq!(t.running_unit_for_thread(thread_id), None);
    }

    #[test]
    fn running_unit_for_thread_returns_none_for_finished_group() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        let uid = UnitId::new(42);
        t.record_spu(uid, gid, 0).unwrap();
        assert_eq!(t.notify_spu_finished(uid), Ok(Some(gid)));
        let thread_id = gid * MAX_SLOTS_PER_GROUP;
        assert_eq!(t.running_unit_for_thread(thread_id), None);
    }

    #[test]
    fn record_spu_increments_remaining() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        t.record_spu(UnitId::new(11), gid, 1).unwrap();
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
    }

    #[test]
    fn record_spu_unknown_group_does_not_corrupt_state() {
        let mut t = ThreadGroupTable::new();
        assert_eq!(
            t.record_spu(UnitId::new(10), 99, 0),
            Err(RecordSpuError::UnknownGroup),
        );
        assert!(t.unit_for_thread(99 * MAX_SLOTS_PER_GROUP).is_none());
    }

    #[test]
    fn record_spu_rejects_finished_group() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Finished;
        assert_eq!(
            t.record_spu(UnitId::new(10), gid, 0),
            Err(RecordSpuError::GroupAlreadyFinished),
        );
    }

    #[test]
    fn record_spu_rejects_duplicate_unit() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        assert_eq!(
            t.record_spu(UnitId::new(10), gid, 1),
            Err(RecordSpuError::DuplicateUnit),
        );
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
    }

    #[test]
    fn record_spu_rejects_thread_id_collision() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        // Different unit, same (group, slot).
        assert_eq!(
            t.record_spu(UnitId::new(11), gid, 0),
            Err(RecordSpuError::ThreadIdCollision),
        );
    }

    #[test]
    fn record_spu_rejects_slot_out_of_range() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        assert_eq!(
            t.record_spu(UnitId::new(10), gid, MAX_SLOTS_PER_GROUP),
            Err(RecordSpuError::SlotOutOfRange),
        );
    }

    #[test]
    fn record_spu_rejects_thread_id_overflow() {
        let mut t = ThreadGroupTable::new();
        // Force a group id that overflows `group_id * 256`.
        t.next_id = 0x0100_0000;
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        assert_eq!(
            t.record_spu(UnitId::new(10), gid, 0),
            Err(RecordSpuError::ThreadIdOverflow),
        );
    }

    #[test]
    fn notify_spu_finished_decrements_and_signals_completion() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        t.record_spu(UnitId::new(11), gid, 1).unwrap();

        assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(None));
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
        assert_eq!(t.get(gid).unwrap().state, GroupState::Running);

        assert_eq!(t.notify_spu_finished(UnitId::new(11)), Ok(Some(gid)));
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 0);
        assert_eq!(t.get(gid).unwrap().state, GroupState::Finished);
    }

    #[test]
    fn notify_unknown_unit_returns_err() {
        let mut t = ThreadGroupTable::new();
        assert_eq!(
            t.notify_spu_finished(UnitId::new(99)),
            Err(NotifySpuFinishedError::UnknownUnit),
        );
    }

    #[test]
    fn notify_double_finish_on_live_group_is_rejected() {
        // The dangerous case: a group with 3 SPUs, unit 10 gets
        // notified twice while 11 and 12 are still running. The
        // old code decremented the counter twice (3 -> 2 -> 1)
        // and would trip the terminal transition one notify
        // early. After moving the unit from `unit_to_group` to
        // `finished_units` on the first notify, the second one
        // returns `AlreadyFinished`.
        let mut t = ThreadGroupTable::new();
        let gid = t.create(3).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        t.record_spu(UnitId::new(11), gid, 1).unwrap();
        t.record_spu(UnitId::new(12), gid, 2).unwrap();
        assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(None));
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
        // Second notify on unit 10 must not decrement the counter
        // again or flip the group's state.
        assert_eq!(
            t.notify_spu_finished(UnitId::new(10)),
            Err(NotifySpuFinishedError::AlreadyFinished { group_id: gid }),
        );
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
        assert_eq!(t.get(gid).unwrap().state, GroupState::Running);
    }

    #[test]
    fn notify_distinguishes_unknown_unit_from_already_finished() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(Some(gid)));
        assert_eq!(
            t.notify_spu_finished(UnitId::new(10)),
            Err(NotifySpuFinishedError::AlreadyFinished { group_id: gid }),
        );
        assert_eq!(
            t.notify_spu_finished(UnitId::new(99)),
            Err(NotifySpuFinishedError::UnknownUnit),
        );
    }

    #[test]
    fn record_spu_rejects_re_registering_finished_unit() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        t.notify_spu_finished(UnitId::new(10)).unwrap();
        // Same unit id, fresh group.
        let gid2 = t.create(1).unwrap();
        t.get_mut(gid2).unwrap().state = GroupState::Running;
        assert_eq!(
            t.record_spu(UnitId::new(10), gid2, 0),
            Err(RecordSpuError::DuplicateUnit),
        );
    }

    #[test]
    fn state_hash_folds_finished_units() {
        // Two tables where one has completed a notify and the
        // other hasn't must hash differently, even though
        // unit_to_group is empty in both (post-move for one,
        // never-populated for the other).
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(1).unwrap();
        let gb = b.create(1).unwrap();
        a.get_mut(ga).unwrap().state = GroupState::Running;
        b.get_mut(gb).unwrap().state = GroupState::Running;
        a.record_spu(UnitId::new(10), ga, 0).unwrap();
        b.record_spu(UnitId::new(10), gb, 0).unwrap();
        a.notify_spu_finished(UnitId::new(10)).unwrap();
        // b has not notified; states diverge.
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn notify_on_created_group_is_rejected() {
        // Finish-before-start used to decrement the counter and
        // leave the group stuck in Created forever.
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        // State stays Created.
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        assert_eq!(
            t.notify_spu_finished(UnitId::new(10)),
            Err(NotifySpuFinishedError::GroupNotRunning {
                state: GroupState::Created
            }),
        );
        // Counter untouched.
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
    }

    #[test]
    fn notify_counter_underflow_is_explicit() {
        // Construct a state where the counter is 0 but the group
        // is still Running (only reachable by direct harness
        // manipulation; normal dispatch cannot produce it). The
        // old saturating_sub path hid this; the new code surfaces
        // it via CounterUnderflow.
        let mut t = ThreadGroupTable::new();
        let gid = t.create(1).unwrap();
        t.record_spu(UnitId::new(10), gid, 0).unwrap();
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.get_mut(gid).unwrap().remaining_unfinished = 0;
        assert_eq!(
            t.notify_spu_finished(UnitId::new(10)),
            Err(NotifySpuFinishedError::CounterUnderflow),
        );
    }

    #[test]
    fn two_groups_independent_completion() {
        let mut t = ThreadGroupTable::new();
        let g1 = t.create(1).unwrap();
        let g2 = t.create(1).unwrap();
        t.get_mut(g1).unwrap().state = GroupState::Running;
        t.get_mut(g2).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), g1, 0).unwrap();
        t.record_spu(UnitId::new(20), g2, 0).unwrap();

        assert_eq!(t.notify_spu_finished(UnitId::new(10)), Ok(Some(g1)));
        assert_eq!(t.get(g2).unwrap().state, GroupState::Running);

        assert_eq!(t.notify_spu_finished(UnitId::new(20)), Ok(Some(g2)));
        assert_eq!(t.get(g2).unwrap().state, GroupState::Finished);
    }
}
