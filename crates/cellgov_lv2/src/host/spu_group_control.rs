//! LV2 dispatch for SPU thread-group control:
//!
//! - terminate a running group;
//! - set and read a group's priority;
//! - read the exit status of a thread;
//! - replace the argument block of a thread.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::GuestStruct;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::thread_group::{ExitStatusError, SetPriorityError, TerminateGroupError};

impl Lv2Host {
    /// `sys_spu_thread_group_terminate`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_TERMINATE`]):
    /// finish every thread of a running group. A join then reads the
    /// `TERMINATED` cause and `value` as the status.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no group has `group_id`.
    /// - `CELL_ESTAT` when the group is not running: not yet started,
    ///   or already finished.
    pub(super) fn dispatch_group_terminate(&mut self, group_id: u32, value: i32) -> Lv2Dispatch {
        match self.state.groups.terminate(group_id, value as u32) {
            Ok(finished_units) => Lv2Dispatch::SpuGroupTerminate {
                group_id,
                finished_units,
            },
            Err(TerminateGroupError::Unknown) => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Err(TerminateGroupError::NotRunning { .. }) => {
                Lv2Dispatch::immediate(errno::CELL_ESTAT.into())
            }
        }
    }

    /// `sys_spu_thread_group_set_priority`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_SET_PRIORITY`]):
    /// store a group's priority. The priority is served back by
    /// `sys_spu_thread_group_get_priority` and is not a scheduling input;
    /// see [`crate::thread_group::ThreadGroupTable::set_priority`].
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no group has `group_id`, checked first.
    /// - `CELL_EINVAL` when the group has no scheduler context or shares
    ///   an SPU with the system, or the priority is out of range for the
    ///   process.
    pub(super) fn dispatch_group_set_priority(
        &mut self,
        group_id: u32,
        priority: i32,
    ) -> Lv2Dispatch {
        let root = self.has_root_perm();
        match self.state.groups.set_priority(group_id, priority, root) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(SetPriorityError::Unknown) => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Err(SetPriorityError::Invalid) => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
        }
    }

    /// `sys_spu_thread_group_get_priority`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_GET_PRIORITY`]):
    /// store a group's priority at `priority_ptr` as an s32: 0 for a
    /// group without a scheduler context, 15 for one that shares an SPU
    /// with the system, and the stored priority otherwise.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no group has `group_id`.
    /// - `CELL_EFAULT` when `priority_ptr` is null.
    pub(super) fn dispatch_group_get_priority(
        &self,
        group_id: u32,
        priority_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(priority) = self.state.groups.priority(group_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if let Some(refusal) = self.efault_if_null(&[priority_ptr]) {
            return refusal;
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(priority_ptr, 4),
                WritePayload::from_slice(&priority.to_be_bytes()),
                requester,
                tick,
            )],
        }
    }

    /// `sys_spu_thread_get_exit_status`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GET_EXIT_STATUS`]):
    /// store the status a thread gave `sys_spu_thread_exit` at
    /// `status_ptr` as an s32.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`.
    /// - `CELL_ESTAT` when the thread did not call
    ///   `sys_spu_thread_exit`. A thread that a group exit or a
    ///   terminate ended has no status either.
    /// - `CELL_EFAULT` when `status_ptr` is null.
    pub(super) fn dispatch_get_exit_status(
        &self,
        thread_id: u32,
        status_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let status = match self.state.groups.thread_exit_status(thread_id) {
            Ok(status) => status,
            Err(ExitStatusError::UnknownThread) => {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
            }
            Err(ExitStatusError::NoStatus) => {
                return Lv2Dispatch::immediate(errno::CELL_ESTAT.into())
            }
        };
        if let Some(refusal) = self.efault_if_null(&[status_ptr]) {
            return refusal;
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(status_ptr, 4),
                WritePayload::from_slice(&status.to_be_bytes()),
                requester,
                tick,
            )],
        }
    }

    /// `sys_spu_thread_set_argument`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_SET_ARGUMENT`]):
    /// replace the argument block of an initialized thread with the 32
    /// bytes at `arg_ptr`. The group's next start passes the new block,
    /// so the call succeeds in every group state.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `arg_ptr` is null or the block is
    ///   unreadable. The dispatch reads the block first: the reference
    ///   console answers EFAULT for a null block on an unknown thread
    ///   (`tests/micro/spu_group_control`).
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`.
    pub(super) fn dispatch_set_argument(
        &mut self,
        thread_id: u32,
        arg_ptr: u32,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if let Some(refusal) = self.efault_if_null(&[arg_ptr]) {
            return refusal;
        }
        let Some(block) = GuestStruct::read(rt, u64::from(arg_ptr), 32) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        if self.state.groups.thread_slot(thread_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let args = std::array::from_fn(|i| block.u64_at(i * 8));
        self.state.groups.set_thread_args(thread_id, args);
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/spu_group_control_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/spu_group_control_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/spu_group_priority_tests.rs"]
mod priority_tests;

#[cfg(test)]
#[path = "tests/spu_group_priority_console_tests.rs"]
mod priority_console_tests;
