//! Managed SPU thread group table.
//!
//! Tracks thread groups created by `sys_spu_thread_group_create`,
//! their slot assignments from `sys_spu_thread_initialize`, and their
//! lifecycle state. Group ids are monotonic u32 tokens starting at 1
//! (0 reserved), allocated deterministically.

use crate::dispatch::{SpuImageHandle, SpuInitState};
use cellgov_event::UnitId;
use std::collections::BTreeMap;

/// A single slot within a thread group, populated by
/// `sys_spu_thread_initialize`.
#[derive(Debug, Clone)]
pub struct ThreadSlot {
    /// Image handle from `sys_spu_image_open`.
    pub image_handle: SpuImageHandle,
    /// SPU thread arguments copied from guest memory at initialize
    /// time. The PPU may reuse the same stack variable for multiple
    /// initialize calls, so args must be captured immediately.
    pub args: [u64; 4],
    /// Initialization state for the SPU (LS bytes, entry, stack, args).
    /// Populated when `sys_spu_thread_group_start` resolves the image.
    pub init: Option<SpuInitState>,
}

/// Lifecycle state of a thread group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Created but not yet started.
    Created,
    /// Started -- SPUs are running.
    Running,
    /// All SPUs finished.
    Finished,
}

/// A managed SPU thread group.
#[derive(Debug, Clone)]
pub struct ThreadGroup {
    /// Group id (monotonic, deterministic).
    pub id: u32,
    /// Number of SPU slots the group expects.
    pub num_threads: u32,
    /// Populated slots, keyed by slot index.
    pub slots: BTreeMap<u32, ThreadSlot>,
    /// Current lifecycle state.
    pub state: GroupState,
    /// SPUs that have not yet finished. Decremented by
    /// `notify_spu_finished`; when it reaches 0 the group
    /// transitions to `Finished`.
    pub remaining_unfinished: u32,
}

/// Table of managed SPU thread groups.
///
/// Groups are created by `sys_spu_thread_group_create`, populated by
/// `sys_spu_thread_initialize`, and started by
/// `sys_spu_thread_group_start`. Group ids are monotonic u32 tokens
/// starting at 1, allocated in creation order.
#[derive(Debug, Clone, Default)]
pub struct ThreadGroupTable {
    groups: BTreeMap<u32, ThreadGroup>,
    /// Maps each registered SPU UnitId to the group it belongs to.
    unit_to_group: BTreeMap<UnitId, u32>,
    /// Maps synthetic thread_id (group_id*256+slot) to runtime UnitId.
    thread_id_to_unit: BTreeMap<u32, UnitId>,
    next_id: u32,
}

impl ThreadGroupTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self {
            groups: BTreeMap::new(),
            unit_to_group: BTreeMap::new(),
            thread_id_to_unit: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Create a new thread group with `num_threads` slots. Returns
    /// the allocated group id.
    pub fn create(&mut self, num_threads: u32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let group = ThreadGroup {
            id,
            num_threads,
            slots: BTreeMap::new(),
            state: GroupState::Created,
            remaining_unfinished: 0,
        };
        self.groups.insert(id, group);
        id
    }

    /// Record a thread slot in a group. `args` are the four u64 words
    /// copied from the guest `sys_spu_thread_argument` struct at
    /// initialize time. Returns `false` if the group does not exist.
    pub fn initialize_thread(
        &mut self,
        group_id: u32,
        slot: u32,
        image_handle: SpuImageHandle,
        args: [u64; 4],
    ) -> bool {
        let group = match self.groups.get_mut(&group_id) {
            Some(g) => g,
            None => return false,
        };
        group.slots.insert(
            slot,
            ThreadSlot {
                image_handle,
                args,
                init: None,
            },
        );
        true
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

    /// Record that `unit_id` is an SPU in `group_id` at `slot` and
    /// increment the group's remaining-unfinished counter. Also
    /// records the synthetic thread_id -> UnitId mapping so
    /// `sys_spu_thread_write_spu_mb` can find the right mailbox.
    pub fn record_spu(&mut self, unit_id: UnitId, group_id: u32, slot: u32) {
        self.unit_to_group.insert(unit_id, group_id);
        let thread_id = group_id * 256 + slot;
        self.thread_id_to_unit.insert(thread_id, unit_id);
        if let Some(group) = self.groups.get_mut(&group_id) {
            group.remaining_unfinished += 1;
        }
    }

    /// Look up the runtime UnitId for a synthetic thread_id.
    pub fn unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        self.thread_id_to_unit.get(&thread_id).copied()
    }

    /// Notify that the SPU with `unit_id` has finished. Decrements
    /// the group's remaining-unfinished counter. Returns `Some(group_id)`
    /// if this was the last unfinished SPU in the group (the group
    /// transitions to `Finished`), or `None` otherwise.
    pub fn notify_spu_finished(&mut self, unit_id: UnitId) -> Option<u32> {
        let group_id = *self.unit_to_group.get(&unit_id)?;
        let group = self.groups.get_mut(&group_id)?;
        group.remaining_unfinished = group.remaining_unfinished.saturating_sub(1);
        if group.remaining_unfinished == 0 && group.state == GroupState::Running {
            group.state = GroupState::Finished;
            Some(group_id)
        } else {
            None
        }
    }

    /// FNV-1a hash of the table for determinism checking.
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
            for (slot_idx, slot) in &group.slots {
                hasher.write(&slot_idx.to_le_bytes());
                hasher.write(&slot.image_handle.raw().to_le_bytes());
            }
        }
        for (uid, gid) in &self.unit_to_group {
            hasher.write(&uid.raw().to_le_bytes());
            hasher.write(&gid.to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_table_is_empty() {
        let t = ThreadGroupTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn create_allocates_ids_starting_at_1() {
        let mut t = ThreadGroupTable::new();
        let g1 = t.create(2);
        let g2 = t.create(4);
        assert_eq!(g1, 1);
        assert_eq!(g2, 2);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn create_records_num_threads() {
        let mut t = ThreadGroupTable::new();
        let id = t.create(3);
        let group = t.get(id).unwrap();
        assert_eq!(group.num_threads, 3);
        assert_eq!(group.state, GroupState::Created);
        assert!(group.slots.is_empty());
    }

    #[test]
    fn initialize_thread_populates_slot() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2);
        let h = SpuImageHandle::new(1);
        assert!(t.initialize_thread(gid, 0, h, [0x2000, 0, 0, 0]));
        let group = t.get(gid).unwrap();
        assert_eq!(group.slots.len(), 1);
        assert_eq!(group.slots[&0].image_handle, h);
        assert_eq!(group.slots[&0].args[0], 0x2000);
    }

    #[test]
    fn initialize_thread_unknown_group_returns_false() {
        let mut t = ThreadGroupTable::new();
        assert!(!t.initialize_thread(99, 0, SpuImageHandle::new(1), [0; 4]));
    }

    #[test]
    fn state_hash_is_deterministic() {
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        let ga = a.create(2);
        let gb = b.create(2);
        a.initialize_thread(ga, 0, SpuImageHandle::new(1), [0; 4]);
        b.initialize_thread(gb, 0, SpuImageHandle::new(1), [0; 4]);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_content() {
        let mut a = ThreadGroupTable::new();
        let mut b = ThreadGroupTable::new();
        a.create(2);
        b.create(3);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_empty_vs_populated_differ() {
        let empty = ThreadGroupTable::new();
        let mut populated = ThreadGroupTable::new();
        populated.create(1);
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn group_id_zero_never_allocated() {
        let mut t = ThreadGroupTable::new();
        for _ in 0..5 {
            let id = t.create(1);
            assert_ne!(id, 0);
        }
    }

    #[test]
    fn record_spu_increments_remaining() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2);
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0);
        t.record_spu(UnitId::new(11), gid, 1);
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 2);
    }

    #[test]
    fn notify_spu_finished_decrements_and_signals_completion() {
        let mut t = ThreadGroupTable::new();
        let gid = t.create(2);
        t.get_mut(gid).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), gid, 0);
        t.record_spu(UnitId::new(11), gid, 1);

        // First SPU finishes -- group not yet done.
        assert_eq!(t.notify_spu_finished(UnitId::new(10)), None);
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 1);
        assert_eq!(t.get(gid).unwrap().state, GroupState::Running);

        // Second SPU finishes -- group done.
        assert_eq!(t.notify_spu_finished(UnitId::new(11)), Some(gid));
        assert_eq!(t.get(gid).unwrap().remaining_unfinished, 0);
        assert_eq!(t.get(gid).unwrap().state, GroupState::Finished);
    }

    #[test]
    fn notify_unknown_unit_returns_none() {
        let mut t = ThreadGroupTable::new();
        assert_eq!(t.notify_spu_finished(UnitId::new(99)), None);
    }

    #[test]
    fn two_groups_independent_completion() {
        let mut t = ThreadGroupTable::new();
        let g1 = t.create(1);
        let g2 = t.create(1);
        t.get_mut(g1).unwrap().state = GroupState::Running;
        t.get_mut(g2).unwrap().state = GroupState::Running;
        t.record_spu(UnitId::new(10), g1, 0);
        t.record_spu(UnitId::new(20), g2, 0);

        // Finishing g1's SPU doesn't affect g2.
        assert_eq!(t.notify_spu_finished(UnitId::new(10)), Some(g1));
        assert_eq!(t.get(g2).unwrap().state, GroupState::Running);

        assert_eq!(t.notify_spu_finished(UnitId::new(20)), Some(g2));
        assert_eq!(t.get(g2).unwrap().state, GroupState::Finished);
    }
}
