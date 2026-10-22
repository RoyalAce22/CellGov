//! Thread slots: initialize, SPU registration, thread-id and unit
//! lookups, and the argument block.

use super::ThreadGroupTable;
use crate::image::SpuImageHandle;
use crate::thread_group::{
    GroupState, InitializeThreadError, RecordSpuError, ThreadSlot, MAX_SLOTS_PER_GROUP,
};
use cellgov_event::UnitId;
use std::collections::BTreeMap;

impl ThreadGroupTable {
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
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(InitializeThreadError::UnknownGroup)?;
        if group.state != GroupState::Created {
            return Err(InitializeThreadError::GroupAlreadyStarted { state: group.state });
        }
        if slot >= MAX_SLOTS_PER_GROUP {
            return Err(InitializeThreadError::SlotOutOfRange);
        }
        if group.slots.contains_key(&slot) {
            return Err(InitializeThreadError::SlotAlreadyInitialized);
        }
        // The slot index is the SPU number, unique within the group;
        // its ceiling is MAX_SLOTS_PER_GROUP. Only the populated count
        // is capped against the group's declared thread count, so any
        // index below that ceiling is accepted: the
        // spu_group_init_limits capture initializes slot 1 of a
        // one-thread group and slot 255, and refuses slot 256.
        if group.slots.len() as u32 >= group.num_threads {
            return Err(InitializeThreadError::GroupFull {
                num_threads: group.num_threads,
            });
        }
        group.slots.insert(
            slot,
            ThreadSlot {
                image_handle,
                args,
                init: None,
                signal_config: 0,
                live_signal_config: None,
                saved_signals: [None; 2],
                event_ports: BTreeMap::new(),
                queue_bindings: BTreeMap::new(),
            },
        );
        Ok(())
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
                .get(group_id)
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
        if self.unit_to_group.contains_key(unit_id) || self.finished_units.contains_key(unit_id) {
            return Err(RecordSpuError::DuplicateUnit);
        }
        if self.thread_id_to_unit.contains_key(thread_id) {
            return Err(RecordSpuError::ThreadIdCollision);
        }
        self.unit_to_group.insert(unit_id, group_id);
        self.thread_id_to_unit.insert(thread_id, unit_id.raw());
        let mut group = self
            .groups
            .get_mut(group_id)
            .expect("group existence checked above and table is &mut");
        group.remaining_unfinished = group
            .remaining_unfinished
            .checked_add(1)
            .expect("remaining_unfinished overflows u32: more than 2^32 SPUs in one group");
        Ok(())
    }

    /// Restore a group whose runtime-side SPU construction failed.
    ///
    /// Returns `false` unless `group_id` names a running group with no
    /// registered units. That restriction keeps this recovery path from
    /// restoring a group after any SPU becomes guest-visible.
    pub(crate) fn cancel_unregistered_start(&mut self, group_id: u32) -> bool {
        let has_units = self.unit_to_group.values().any(|&gid| gid == group_id)
            || self.finished_units.values().any(|&gid| gid == group_id);
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        if group.state != GroupState::Running || group.remaining_unfinished != 0 || has_units {
            return false;
        }
        group.state = GroupState::Created;
        true
    }

    /// Look up the runtime UnitId for a synthetic thread_id.
    pub fn unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        self.thread_id_to_unit
            .get(thread_id)
            .map(|&raw| UnitId::new(raw))
    }

    /// The slot an initialized thread occupies, as `(group_id, slot)`.
    pub(super) fn slot_of_thread(&self, thread_id: u32) -> Option<(u32, u32)> {
        let (group_id, slot) = (
            thread_id / MAX_SLOTS_PER_GROUP,
            thread_id % MAX_SLOTS_PER_GROUP,
        );
        self.groups
            .get(group_id)?
            .slots
            .contains_key(&slot)
            .then_some((group_id, slot))
    }

    /// The `(group_id, slot)` of an initialized thread, or `None` when
    /// no initialized thread has the id.
    pub fn thread_slot(&self, thread_id: u32) -> Option<(u32, u32)> {
        self.slot_of_thread(thread_id)
    }

    /// The group of a registered SPU that has not finished.
    pub fn live_group_of(&self, unit_id: UnitId) -> Option<u32> {
        self.unit_to_group.get(unit_id).copied()
    }

    /// The group of a registered SPU, finished or not.
    pub fn group_of(&self, unit_id: UnitId) -> Option<u32> {
        self.unit_to_group
            .get(unit_id)
            .or_else(|| self.finished_units.get(unit_id))
            .copied()
    }

    /// Thread-id lookup filtered to [`GroupState::Running`].
    ///
    /// Mailbox writes and other state-sensitive paths must use this
    /// so writes to not-yet-started or already-finished threads
    /// surface as `ESRCH` rather than queuing in a dead mailbox.
    pub fn running_unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        let unit_id = self.unit_for_thread(thread_id)?;
        let group_id = self.unit_to_group.get(unit_id)?;
        let group = self.groups.get(*group_id)?;
        if group.state == GroupState::Running {
            Some(unit_id)
        } else {
            None
        }
    }

    /// The thread id of a registered SPU that has not finished.
    pub fn live_thread_of(&self, unit_id: UnitId) -> Option<u32> {
        self.live_group_of(unit_id)?;
        self.thread_id_to_unit
            .iter()
            .find(|&(_, &raw)| raw == unit_id.raw())
            .map(|(thread_id, _)| thread_id)
    }

    /// The units of group `group_id` that have not finished.
    pub fn live_units(&self, group_id: u32) -> Vec<UnitId> {
        self.unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect()
    }

    /// Replace the argument block of an initialized thread. The group's
    /// next start passes it; a running SPU keeps the block it started
    /// with. Returns `false` when no initialized thread has the id.
    pub fn set_thread_args(&mut self, thread_id: u32, args: [u64; 4]) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        match group.slots.get_mut(&slot) {
            Some(s) => {
                s.args = args;
                true
            }
            None => false,
        }
    }
}
