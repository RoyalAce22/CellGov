//! The SPE problem-state operations another processor performs on a
//! registered unit: run control, status, next PC, the mailboxes and
//! the signal-notification registers.

use cellgov_event::UnitId;
use cellgov_exec::{ProblemStateError, SignalNotifier, StallWake, UnitStatus};
use cellgov_sync::MailboxId;

use super::Runtime;

impl Runtime {
    /// The `SPU_Status` word of `unit`, or `None` for a unit without SPE
    /// problem-state registers or no unit with the id.
    ///
    /// A unit CellGov refused reports R clear with no stop cause.
    pub fn unit_spu_status(&self, unit: UnitId) -> Option<u32> {
        let status = self.registry.get(unit)?.spu_status()?;
        if self.registry.effective_status(unit) == Some(UnitStatus::Faulted) {
            return Some(0);
        }
        Some(status)
    }

    /// An `SPU_RunCntl` stop request for `unit`. A unit parked on a
    /// blocked channel reports that in its status, and a wake queued for
    /// it no longer schedules it. A unit CellGov refused keeps its
    /// refusal.
    ///
    /// [CBEA p:92 s:8.5.1] run control 00 is a stop request: no further instructions issue.
    /// [CBEA p:94 s:8.5.2] an SPU stopped while it waits on a blocked channel sets W with the stopped status.
    ///
    /// # Errors
    ///
    /// - [`ProblemStateError::UnknownUnit`] when no unit has the id.
    /// - [`ProblemStateError::Retired`] when a `Finished` status override
    ///   holds the unit.
    /// - [`ProblemStateError::NoProblemState`] for a unit without SPE
    ///   problem-state registers.
    pub fn request_unit_stop(&mut self, unit: UnitId) -> Result<(), ProblemStateError> {
        self.refuse_retired(unit)?;
        let status = self.registry.effective_status(unit);
        if status == Some(UnitStatus::Faulted) {
            return Ok(());
        }
        let waiting = status == Some(UnitStatus::Blocked);
        self.registry
            .get_mut(unit)
            .ok_or(ProblemStateError::UnknownUnit)?
            .request_stop(waiting)?;
        self.registry.clear_status_override(unit);
        Ok(())
    }

    /// Write `SPU_NPC` of a stopped `unit`. A parked channel access
    /// took nothing, so the restart at the new address abandons it with
    /// no channel state to return.
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::UnknownUnit`], [`ProblemStateError::Retired`]
    /// and [`ProblemStateError::NoProblemState`] as for
    /// [`Runtime::request_unit_stop`]; [`ProblemStateError::Running`]
    /// while the unit runs; [`ProblemStateError::Refused`] for a unit
    /// CellGov refused.
    pub fn write_unit_npc(&mut self, unit: UnitId, npc: u32) -> Result<(), ProblemStateError> {
        self.refuse_retired(unit)?;
        if self.registry.effective_status(unit) == Some(UnitStatus::Faulted) {
            return Err(ProblemStateError::Refused);
        }
        self.registry
            .get_mut(unit)
            .ok_or(ProblemStateError::UnknownUnit)?
            .write_npc(npc)
    }

    /// Write one signal-notification register of `unit`.
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::UnknownUnit`], [`ProblemStateError::Retired`]
    /// and [`ProblemStateError::NoProblemState`] as for
    /// [`Runtime::request_unit_stop`].
    pub fn write_unit_signal(
        &mut self,
        unit: UnitId,
        register: SignalNotifier,
        value: u32,
    ) -> Result<(), ProblemStateError> {
        self.refuse_retired(unit)?;
        self.registry
            .get_mut(unit)
            .ok_or(ProblemStateError::UnknownUnit)?
            .write_signal(register, value)
    }

    /// Read `SPU_Out_Mbox` of `unit`: the message it wrote, or `None`
    /// when the mailbox is empty. A unit parked writing the mailbox
    /// becomes runnable and runs its write again.
    ///
    /// [CBEA p:98 s:8.6.1] a write to a full outbound mailbox stalls the SPU until another processor reads it.
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::UnknownUnit`], [`ProblemStateError::Retired`]
    /// and [`ProblemStateError::NoProblemState`] as for
    /// [`Runtime::request_unit_stop`].
    pub fn read_unit_out_mbox(&mut self, unit: UnitId) -> Result<Option<u32>, ProblemStateError> {
        self.refuse_retired(unit)?;
        let message = self
            .registry
            .get_mut(unit)
            .ok_or(ProblemStateError::UnknownUnit)?
            .read_out_mbox()?;
        if message.is_some() && self.stall_ends(unit, StallWake::OutboundMailboxRead, false) {
            self.registry
                .set_status_override(unit, UnitStatus::Runnable);
        }
        Ok(message)
    }

    /// Write `SPU_In_Mbox` of `unit`. A write to a full mailbox
    /// replaces its oldest message (CellGov's choice of which message
    /// is lost), and a unit parked on the mailbox becomes runnable.
    ///
    /// [CBEA p:99 s:8.6.2] an MMIO write puts 32 bits into the SPU inbound mailbox queue.
    /// [CBE-Handbook p:541 s:19.6.6.2] a PPE write to a full inbound mailbox does not stall; a message is lost.
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::UnknownUnit`], [`ProblemStateError::Retired`]
    /// and [`ProblemStateError::NoProblemState`] as for
    /// [`Runtime::request_unit_stop`]. A unit without an inbound mailbox
    /// takes the `NoProblemState` refusal.
    pub fn write_unit_in_mbox(
        &mut self,
        unit: UnitId,
        message: u32,
    ) -> Result<(), ProblemStateError> {
        self.refuse_retired(unit)?;
        let spu = self
            .registry
            .get(unit)
            .ok_or(ProblemStateError::UnknownUnit)?;
        if spu.spu_status().is_none()
            || !self.deliver_mailbox_message(MailboxId::new(unit.raw()), message)
        {
            return Err(ProblemStateError::NoProblemState);
        }
        Ok(())
    }

    /// Put `message` in `mailbox` and make a unit parked on it runnable.
    /// An SPU's inbound mailbox shares its unit id. `false` when no
    /// mailbox has the id. A parked unit that names no channel stall
    /// wakes too, as before channel stalls had names.
    pub(super) fn deliver_mailbox_message(&mut self, mailbox: MailboxId, message: u32) -> bool {
        {
            let Some(mut queue) = self.mailbox_registry.get_mut(mailbox) else {
                return false;
            };
            queue.force_send(message);
        }
        let target = UnitId::new(mailbox.raw());
        if self.stall_ends(target, StallWake::MailboxDelivery, true) {
            self.registry
                .set_status_override(target, UnitStatus::Runnable);
        }
        true
    }

    /// Whether `wake` ends the park of `unit`: the unit is blocked and
    /// its channel stall names `wake`. `unstalled` answers for a blocked
    /// unit that names no channel stall.
    pub(super) fn stall_ends(&self, unit: UnitId, wake: StallWake, unstalled: bool) -> bool {
        self.registry.effective_status(unit) == Some(UnitStatus::Blocked)
            && self
                .registry
                .get(unit)
                .and_then(|unit| unit.channel_stall())
                .map_or(unstalled, |stall| stall.wake == wake)
    }

    fn refuse_retired(&self, unit: UnitId) -> Result<(), ProblemStateError> {
        if self.registry.status_override(unit) == Some(UnitStatus::Finished) {
            return Err(ProblemStateError::Retired);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/problem_state_tests.rs"]
mod tests;
