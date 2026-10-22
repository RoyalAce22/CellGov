//! Group create and destroy, the SPU pool split and limits, priority,
//! and group lookup.

use super::ThreadGroupTable;
use crate::thread_group::types::{priority_in_range, PoolDemand};
use crate::thread_group::{
    CreateGroupError, DestroyGroupError, GroupKind, GroupRequest, GroupState, SetLimitsError,
    SetPriorityError, ThreadGroup, MAX_SLOTS_PER_GROUP,
};
use cellgov_mem::lanes::LaneEntryMut;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
use cellgov_ps3_abi::lv2::spu::{group_join_cause, group_priority, group_type, pool};
use std::collections::BTreeMap;

impl ThreadGroupTable {
    /// Create a normal group of the boot process with `num_threads`
    /// slots, outside the SPU pool accounting.
    ///
    /// Returns `None` if the u32 id space is exhausted.
    pub fn create(&mut self, num_threads: u32) -> Option<u32> {
        self.insert_group(
            GroupRequest {
                num_threads,
                group_type: group_type::NORMAL,
                priority: 0,
            },
            BOOT_PROCESS_PID,
        )
    }

    /// Create the group `request` describes for process `pid`, with root
    /// permission when `root`, if that process's SPU pool has room for
    /// it.
    ///
    /// # Errors
    /// See [`CreateGroupError`]; an error creates nothing.
    pub fn create_typed(
        &mut self,
        request: GroupRequest,
        root: bool,
        pid: u32,
    ) -> Result<u32, CreateGroupError> {
        self.check_valid(request, root, pid)?;
        let demand = PoolDemand::of(GroupKind::of_type(request.group_type), request.num_threads);
        if !self.occupied(pid).with(demand).fits(self.spu_limit(pid)) {
            return Err(CreateGroupError::Busy);
        }
        self.insert_group(request, pid)
            .ok_or(CreateGroupError::Exhausted)
    }

    /// Whether process `pid`, with root permission when `root`, may
    /// create the group `request` describes, and whether the group can
    /// ever fit that process's pool, whatever groups exist.
    ///
    /// # Errors
    /// [`CreateGroupError::Invalid`] when it cannot.
    pub fn check_valid(
        &self,
        request: GroupRequest,
        root: bool,
        pid: u32,
    ) -> Result<(), CreateGroupError> {
        let GroupRequest {
            num_threads,
            group_type,
            priority,
        } = request;
        let root_type = group_type::ROOT_TYPES.contains(&group_type);
        if !(group_type::USER_TYPES.contains(&group_type) || (root && root_type)) {
            return Err(CreateGroupError::Invalid);
        }
        let max_threads = if root_type && group_type & group_type::SINGLE_THREAD_MASK != 0 {
            1
        } else {
            pool::USABLE_SPUS
        };
        let kind = GroupKind::of_type(group_type);
        let demand = PoolDemand::of(kind, num_threads);
        let spu_limit = self.spu_limit(pid);
        if num_threads > max_threads
            || (kind == GroupKind::Context && !priority_in_range(priority, root))
            || demand.physical > spu_limit
            || demand.controllable > spu_limit
            || (kind == GroupKind::CooperateWithSystem && num_threads < 2)
        {
            return Err(CreateGroupError::Invalid);
        }
        Ok(())
    }

    /// Give the group `group_id` priority `priority`, as
    /// `sys_spu_thread_group_set_priority` does for a process with root
    /// permission when `root`. The priority is served back and is not a
    /// scheduling input: CellGov runs every SPU thread as its own unit, so
    /// the contest for SPUs it decides on the console never arises.
    ///
    /// # Errors
    /// See [`SetPriorityError`]; an error keeps the old priority.
    pub fn set_priority(
        &mut self,
        group_id: u32,
        priority: i32,
        root: bool,
    ) -> Result<(), SetPriorityError> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(SetPriorityError::Unknown)?;
        if group.kind() != GroupKind::Context || !priority_in_range(priority, root) {
            return Err(SetPriorityError::Invalid);
        }
        group.priority = priority;
        Ok(())
    }

    /// The priority `sys_spu_thread_group_get_priority` reads for the
    /// group `group_id`: 0 for a group without a scheduler context,
    /// [`group_priority::COOPERATING`] for one that shares an SPU with the
    /// system, and the stored priority otherwise. `None` when no group
    /// has the id.
    pub fn priority(&self, group_id: u32) -> Option<i32> {
        let group = self.groups.get(group_id)?;
        Some(match group.kind() {
            GroupKind::NonContext => 0,
            GroupKind::CooperateWithSystem => group_priority::COOPERATING,
            GroupKind::Context => group.priority,
        })
    }

    /// The SPUs left for process `pid`'s SPU threads once its Raw SPUs
    /// take their share.
    pub fn spu_limit(&self, pid: u32) -> u32 {
        pool::USABLE_SPUS - self.raw_limit(pid)
    }

    /// The physical SPUs process `pid` set aside for Raw SPUs.
    pub fn raw_limit(&self, pid: u32) -> u32 {
        self.raw_limits.get(pid).copied().unwrap_or(0)
    }

    /// Set aside `max_raw` physical SPUs for process `pid`'s Raw SPUs, as
    /// `sys_spu_initialize` does, while `live_raw` Raw SPUs exist.
    ///
    /// # Errors
    /// See [`SetLimitsError`]; an error keeps the old split.
    // The console capture of tests/micro/spu_raw_lifecycle answers
    // CELL_EBUSY to a limit of 1 while two Raw SPUs exist.
    pub fn set_raw_limit(
        &mut self,
        pid: u32,
        max_raw: u32,
        live_raw: u32,
    ) -> Result<(), SetLimitsError> {
        if max_raw > pool::RAW_SPU_MAX {
            return Err(SetLimitsError::Invalid { max_raw });
        }
        if live_raw > max_raw || !self.occupied(pid).fits(pool::USABLE_SPUS - max_raw) {
            return Err(SetLimitsError::Busy { max_raw });
        }
        if max_raw == 0 {
            self.raw_limits.remove(pid);
        } else {
            self.raw_limits.insert(pid, max_raw);
        }
        Ok(())
    }

    /// What every group of process `pid` needs from its SPU pool. A
    /// group keeps its share until its destroy.
    fn occupied(&self, pid: u32) -> PoolDemand {
        self.groups
            .values()
            .filter(|group| group.pid == pid)
            .fold(PoolDemand::default(), |acc, group| {
                acc.with(PoolDemand::of(group.kind(), group.num_threads))
            })
    }

    fn insert_group(&mut self, request: GroupRequest, pid: u32) -> Option<u32> {
        let GroupRequest {
            num_threads,
            group_type,
            priority,
        } = request;
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
            join_cause: group_join_cause::ALL_THREADS_EXIT,
            exit_status: 0,
            thread_exit_status: BTreeMap::new(),
            event_queues: BTreeMap::new(),
            group_type,
            priority,
            suspended: false,
            previous_exit_status: BTreeMap::new(),
            container: None,
            pid,
        };
        self.groups.insert(id, group);
        Some(id)
    }

    /// Look up a group by id.
    pub fn get(&self, group_id: u32) -> Option<&ThreadGroup> {
        self.groups.get(group_id)
    }

    /// Mutably look up a group by id.
    pub fn get_mut(&mut self, group_id: u32) -> Option<LaneEntryMut<'_, u32, ThreadGroup>> {
        self.groups.get_mut(group_id)
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
        match self.groups.get(group_id) {
            None => return Err(DestroyGroupError::Unknown),
            Some(g) if g.state == GroupState::Running => return Err(DestroyGroupError::Busy),
            Some(_) => {}
        }
        self.groups.remove(group_id);
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
}
