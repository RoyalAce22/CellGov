//! Managed SPU thread group table.
//!
//! Owns group lifecycle from create through finish. Group ids are
//! monotonic u32 tokens starting at 1; 0 is reserved.

use crate::dispatch::SpuInitState;
use crate::image::SpuImageHandle;
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
    /// Snapshot of the argument block at initialize time; the PPU
    /// may reuse the same stack variable across initialize calls.
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
    /// Drives the terminal transition to [`GroupState::Finished`]
    /// when it reaches 0.
    pub remaining_unfinished: u32,
}

/// Failure modes of [`ThreadGroupTable::initialize_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InitializeThreadError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Slots can only be populated while the group is
    /// [`GroupState::Created`].
    #[error("thread group already started (state {state:?})")]
    GroupAlreadyStarted {
        /// Current group state.
        state: GroupState,
    },
    /// Slot index is `>= num_threads` declared at create time.
    #[error("slot {slot} out of bounds (group declared {num_threads} threads)")]
    SlotOutOfBounds {
        /// The rejected slot index.
        slot: u32,
        /// The group's declared slot count.
        num_threads: u32,
    },
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// The slot already has an entry.
    #[error("slot already initialized")]
    SlotAlreadyInitialized,
}

/// Failure modes of [`ThreadGroupTable::record_spu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordSpuError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Registering against a finished group would inflate the
    /// counter without triggering the terminal transition.
    #[error("thread group already finished")]
    GroupAlreadyFinished,
    /// `unit_id` is already registered in some group, or was
    /// registered in one that has since finished.
    #[error("SPU unit already registered")]
    DuplicateUnit,
    /// `(group_id, slot)` already maps to a different unit.
    #[error("(group, slot) already maps to a different unit")]
    ThreadIdCollision,
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// `group_id * 256 + slot` overflows `u32`.
    #[error("group_id * 256 + slot overflowed u32")]
    ThreadIdOverflow,
}

/// Failure modes of [`ThreadGroupTable::notify_spu_finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotifySpuFinishedError {
    /// `unit_id` was never registered as an SPU. Benign for
    /// non-SPU units; callers iterating every unit can ignore.
    #[error("unknown SPU unit")]
    UnknownUnit,
    /// `unit_id` was registered and its notify already fired.
    /// Distinct from [`Self::UnknownUnit`] so a double-notify
    /// against a live unit does not masquerade as a
    /// non-SPU-unit call.
    #[error("SPU notify already fired (group {group_id})")]
    AlreadyFinished {
        /// Group this unit belonged to.
        group_id: u32,
    },
    /// Owning group is not in [`GroupState::Running`]
    /// (finish-before-start).
    #[error("thread group not running (state {state:?})")]
    GroupNotRunning {
        /// Current group state.
        state: GroupState,
    },
    /// `remaining_unfinished` was already 0.
    #[error("remaining_unfinished was already 0")]
    CounterUnderflow,
}

/// Failure modes of [`ThreadGroupTable::destroy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DestroyGroupError {
    /// Group id does not exist (`sys_spu_thread_group_destroy` -> CELL_ESRCH).
    #[error("unknown thread group")]
    Unknown,
    /// Group is in [`GroupState::Running`]; the title must terminate
    /// or join it first (`sys_spu_thread_group_destroy` -> CELL_EBUSY).
    #[error("thread group is running")]
    Busy,
}

/// Table of managed SPU thread groups.
///
/// `unit_to_group` and `finished_units` partition the SPU
/// registration space: a unit lives in exactly one of them, and
/// `notify_spu_finished` moves entries from the former to the
/// latter. Keeping them distinct is what lets a double-notify
/// surface as [`NotifySpuFinishedError::AlreadyFinished`] rather
/// than re-decrementing the counter.
#[derive(Debug, Clone, Default)]
pub struct ThreadGroupTable {
    groups: BTreeMap<u32, ThreadGroup>,
    unit_to_group: BTreeMap<UnitId, u32>,
    finished_units: BTreeMap<UnitId, u32>,
    /// Synthetic `group_id * 256 + slot` -> runtime `UnitId`.
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
    ///
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

    /// Withdraw a group whose state allows destruction.
    ///
    /// Mirrors `sys_spu_thread_group_destroy`: a group in
    /// [`GroupState::Running`] is in-flight and reports
    /// [`DestroyGroupError::Busy`]; an unknown id reports
    /// [`DestroyGroupError::Unknown`]. The unit / thread-id maps for
    /// the group's slots are scrubbed so a future `create` reusing
    /// the same id starts clean.
    pub fn destroy(&mut self, group_id: u32) -> Result<(), DestroyGroupError> {
        match self.groups.get(&group_id) {
            None => return Err(DestroyGroupError::Unknown),
            Some(g) if g.state == GroupState::Running => return Err(DestroyGroupError::Busy),
            Some(_) => {}
        }
        self.groups.remove(&group_id);
        self.unit_to_group.retain(|_, gid| *gid != group_id);
        self.finished_units.retain(|_, gid| *gid != group_id);
        self.thread_id_to_unit
            .retain(|tid, _| tid / MAX_SLOTS_PER_GROUP != group_id);
        Ok(())
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
    /// All validation runs before any insert, so `Err` leaves the
    /// table untouched.
    ///
    /// # Errors
    /// See [`RecordSpuError`].
    pub fn record_spu(
        &mut self,
        unit_id: UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), RecordSpuError> {
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
    /// Mailbox writes and other state-sensitive paths must use this
    /// so writes to not-yet-started or already-finished threads
    /// surface as `ESRCH` rather than queuing in a dead mailbox.
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
        // Move across the unit_to_group/finished_units boundary so
        // a second notify hits AlreadyFinished, not re-decrement.
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
    /// `unit_to_group`, `finished_units`, and `thread_id_to_unit`
    /// are each folded independently: they are separately mutable,
    /// so a desync between them is only catchable by hashing each
    /// on its own.
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
#[path = "tests/thread_group_tests.rs"]
mod tests;
