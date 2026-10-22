//! Run control: SPU finish and exit, group exit and terminate,
//! suspend, resume, yield, restart, and the join and exit-status reads.

use super::ThreadGroupTable;
#[cfg(doc)]
use crate::thread_group::ThreadGroup;
use crate::thread_group::{
    ExitStatusError, GroupState, NotifySpuFinishedError, RunControlError, TerminateGroupError,
    MAX_SLOTS_PER_GROUP,
};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::spu::group_join_cause;

impl ThreadGroupTable {
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
        if let Some(&group_id) = self.finished_units.get(unit_id) {
            return Err(NotifySpuFinishedError::AlreadyFinished { group_id });
        }
        let group_id = *self
            .unit_to_group
            .get(unit_id)
            .ok_or(NotifySpuFinishedError::UnknownUnit)?;
        // Existence is guaranteed: `record_spu` checks the group
        // before inserting into `unit_to_group`, and the table
        // exposes no group-destroy path.
        let mut group = self
            .groups
            .get_mut(group_id)
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
        self.unit_to_group.remove(unit_id);
        self.finished_units.insert(unit_id, group_id);
        if terminal {
            Ok(Some(group_id))
        } else {
            Ok(None)
        }
    }

    /// Record that the SPU `unit_id` called `sys_spu_thread_exit` with
    /// `status`, then notify its finish as
    /// [`Self::notify_spu_finished`] does.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]; an error records nothing.
    pub fn thread_exit(
        &mut self,
        unit_id: UnitId,
        status: u32,
    ) -> Result<Option<u32>, NotifySpuFinishedError> {
        let group_id = self.unit_to_group.get(unit_id).copied();
        let finished = self.notify_spu_finished(unit_id)?;
        if let Some(mut group) = group_id.and_then(|gid| self.groups.get_mut(gid)) {
            group.thread_exit_status.insert(unit_id.raw(), status);
        }
        Ok(finished)
    }

    /// End the group of the SPU `unit_id`, which called
    /// `sys_spu_thread_group_exit` with `status`. Every thread of the
    /// group finishes, and a join reads [`group_join_cause::GROUP_EXIT`]
    /// with `status`. Returns the group id and the group's other units
    /// that had not finished.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]; an error changes nothing.
    pub fn group_exit(
        &mut self,
        unit_id: UnitId,
        status: u32,
    ) -> Result<(u32, Vec<UnitId>), NotifySpuFinishedError> {
        if let Some(&group_id) = self.finished_units.get(unit_id) {
            return Err(NotifySpuFinishedError::AlreadyFinished { group_id });
        }
        let group_id = *self
            .unit_to_group
            .get(unit_id)
            .ok_or(NotifySpuFinishedError::UnknownUnit)?;
        // `destroy` scrubs a group's units with it, so a miss here
        // answers as the unit being unknown.
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return Err(NotifySpuFinishedError::UnknownUnit);
        };
        if group.state != GroupState::Running {
            return Err(NotifySpuFinishedError::GroupNotRunning { state: group.state });
        }
        group.state = GroupState::Finished;
        group.remaining_unfinished = 0;
        group.join_cause = group_join_cause::GROUP_EXIT;
        group.exit_status = status;
        let members: Vec<UnitId> = self
            .unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect();
        for &unit in &members {
            self.unit_to_group.remove(unit);
            self.finished_units.insert(unit, group_id);
        }
        let others = members
            .into_iter()
            .filter(|&unit| unit != unit_id)
            .collect();
        Ok((group_id, others))
    }

    /// End the running group `group_id`, as `sys_spu_thread_group_terminate` does.
    ///
    /// Every thread of the group finishes, and a join reads
    /// [`group_join_cause::TERMINATED`] with `status`. A terminated
    /// thread records no exit status. Returns the units of the group
    /// that were still running.
    ///
    /// # Errors
    /// See [`TerminateGroupError`]; an error changes nothing.
    pub fn terminate(
        &mut self,
        group_id: u32,
        status: u32,
    ) -> Result<Vec<UnitId>, TerminateGroupError> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(TerminateGroupError::Unknown)?;
        if group.state != GroupState::Running {
            return Err(TerminateGroupError::NotRunning { state: group.state });
        }
        group.state = GroupState::Finished;
        group.suspended = false;
        group.remaining_unfinished = 0;
        group.join_cause = group_join_cause::TERMINATED;
        group.exit_status = status;
        let members: Vec<UnitId> = self
            .unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect();
        for &unit in &members {
            self.unit_to_group.remove(unit);
            self.finished_units.insert(unit, group_id);
        }
        Ok(members)
    }

    /// Suspend the running group `group_id`, as
    /// `sys_spu_thread_group_suspend` does. A suspended group stays
    /// suspended. Returns the group's units that have not finished, for
    /// the runtime to hold.
    ///
    /// # Errors
    /// See [`RunControlError`]; an error changes nothing.
    pub fn suspend(&mut self, group_id: u32) -> Result<Vec<UnitId>, RunControlError> {
        {
            let mut group = self
                .groups
                .get_mut(group_id)
                .ok_or(RunControlError::Unknown)?;
            if !group.takes_run_control() {
                return Err(RunControlError::NoRunControl);
            }
            if group.state != GroupState::Running {
                return Err(RunControlError::NotRunning);
            }
            group.suspended = true;
        }
        Ok(self.live_units(group_id))
    }

    /// Resume the suspended group `group_id`, as
    /// `sys_spu_thread_group_resume` does. Returns the group's units that
    /// have not finished, for the runtime to release.
    ///
    /// # Errors
    /// See [`RunControlError`]; [`RunControlError::NotRunning`] also
    /// covers a running group that is not suspended. An error changes
    /// nothing.
    pub fn resume(&mut self, group_id: u32) -> Result<Vec<UnitId>, RunControlError> {
        {
            let mut group = self
                .groups
                .get_mut(group_id)
                .ok_or(RunControlError::Unknown)?;
            if !group.takes_run_control() {
                return Err(RunControlError::NoRunControl);
            }
            if group.state != GroupState::Running || !group.suspended {
                return Err(RunControlError::NotRunning);
            }
            group.suspended = false;
        }
        Ok(self.live_units(group_id))
    }

    /// Check a `sys_spu_thread_group_yield` of group `group_id`. A yield
    /// changes nothing: every SPU thread runs as its own unit, so no other
    /// group waits for the SPUs it would give up.
    ///
    /// # Errors
    /// [`RunControlError::Unknown`], and [`RunControlError::NotRunning`]
    /// when the group is not running or is suspended. A group that takes
    /// no run control answers `Ok`.
    pub fn check_yield(&self, group_id: u32) -> Result<(), RunControlError> {
        let group = self.groups.get(group_id).ok_or(RunControlError::Unknown)?;
        if !group.takes_run_control() {
            return Ok(());
        }
        if group.state != GroupState::Running || group.suspended {
            return Err(RunControlError::NotRunning);
        }
        Ok(())
    }

    /// Return the finished group `group_id` to [`GroupState::Created`]
    /// for another start, whether or not a join has read it: its slots
    /// and their arguments stay, each thread's exit status moves to
    /// [`ThreadGroup::previous_exit_status`], and the previous run's
    /// units and join record go. Returns `false`, changing nothing,
    /// unless the group finished.
    pub(crate) fn reset_for_restart(&mut self, group_id: u32) -> bool {
        let units: Vec<(u32, u64)> = self
            .thread_id_to_unit
            .iter()
            .filter(|(tid, _)| tid / MAX_SLOTS_PER_GROUP == group_id)
            .map(|(tid, &raw)| (tid % MAX_SLOTS_PER_GROUP, raw))
            .collect();
        {
            let Some(mut group) = self.groups.get_mut(group_id) else {
                return false;
            };
            if group.state != GroupState::Finished {
                return false;
            }
            for (slot, raw) in units {
                if let Some(&status) = group.thread_exit_status.get(&raw) {
                    group.previous_exit_status.insert(slot, status);
                }
            }
            group.state = GroupState::Created;
            group.remaining_unfinished = 0;
            group.join_cause = group_join_cause::ALL_THREADS_EXIT;
            group.exit_status = 0;
            group.thread_exit_status.clear();
        }
        self.finished_units.retain(|_, gid| *gid != group_id);
        self.thread_id_to_unit
            .retain(|tid, _| tid / MAX_SLOTS_PER_GROUP != group_id);
        true
    }

    /// The status an initialized thread gave `sys_spu_thread_exit`.
    ///
    /// # Errors
    /// See [`ExitStatusError`].
    pub fn thread_exit_status(&self, thread_id: u32) -> Result<u32, ExitStatusError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(ExitStatusError::UnknownThread)?;
        let group = self
            .groups
            .get(group_id)
            .ok_or(ExitStatusError::UnknownThread)?;
        self.unit_for_thread(thread_id)
            .and_then(|unit| group.thread_exit_status.get(&unit.raw()))
            .or_else(|| group.previous_exit_status.get(&slot))
            .copied()
            .ok_or(ExitStatusError::NoStatus)
    }

    /// The `cause` and `status` a join of the finished group
    /// `group_id` reads, or `None` for a group that is not finished.
    pub fn join_result(&self, group_id: u32) -> Option<(u32, u32)> {
        let group = self.groups.get(group_id)?;
        (group.state == GroupState::Finished).then_some((group.join_cause, group.exit_status))
    }
}
