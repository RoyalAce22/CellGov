//! Child-thread stacks and SPU group notes.

use crate::ppu_thread::{place_child_stack, ThreadStack};

use super::model::Lv2Host;

impl Lv2Host {
    /// Place a child-thread stack of `size` bytes at `align` in the
    /// lowest range no thread in the table holds; `None` when no range
    /// of the child-stack arena fits.
    ///
    /// # Cross-module contract
    ///
    /// Only a thread in the thread table holds a block.
    /// `sys_ppu_thread_create` enters the thread in the same step that
    /// places the block, so two creates never get one block. A refused
    /// create enters no thread, and its block is free again with no
    /// call.
    pub fn allocate_child_stack(&self, size: u64, align: u64) -> Option<ThreadStack> {
        place_child_stack(self.state.ppu_threads.stacks(), size, align)
    }

    /// Bind an SPU `unit_id` to `(group_id, slot)`.
    pub fn record_spu(
        &mut self,
        unit_id: cellgov_event::UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), crate::thread_group::RecordSpuError> {
        self.state.groups.record_spu(unit_id, group_id, slot)
    }

    /// Restore a group after its runtime factory refused an image.
    pub fn cancel_unregistered_spu_group_start(&mut self, group_id: u32) -> bool {
        self.state.groups.cancel_unregistered_start(group_id)
    }

    /// `Ok(Some(group_id))` when this notify drove the group to
    /// `Finished`.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: cellgov_event::UnitId,
    ) -> Result<Option<u32>, crate::thread_group::NotifySpuFinishedError> {
        self.state.groups.notify_spu_finished(unit_id)
    }

    /// The group of an SPU thread that has not finished, or `None` for
    /// a unit no thread group holds.
    pub fn live_spu_group(&self, unit_id: cellgov_event::UnitId) -> Option<u32> {
        self.state.groups.live_group_of(unit_id)
    }

    /// The SPU thread `unit_id` called `sys_spu_thread_exit` with
    /// `status`. See [`crate::thread_group::ThreadGroupTable::thread_exit`].
    pub fn spu_thread_exit(
        &mut self,
        unit_id: cellgov_event::UnitId,
        status: u32,
    ) -> Result<Option<u32>, crate::thread_group::NotifySpuFinishedError> {
        self.state.groups.thread_exit(unit_id, status)
    }

    /// The SPU thread `unit_id` called `sys_spu_thread_group_exit` with
    /// `status`. See [`crate::thread_group::ThreadGroupTable::group_exit`].
    pub fn spu_group_exit(
        &mut self,
        unit_id: cellgov_event::UnitId,
        status: u32,
    ) -> Result<(u32, Vec<cellgov_event::UnitId>), crate::thread_group::NotifySpuFinishedError>
    {
        self.state.groups.group_exit(unit_id, status)
    }

    /// The `cause` and `status` a join of the finished group reads.
    pub fn spu_group_join_result(&self, group_id: u32) -> Option<(u32, u32)> {
        self.state.groups.join_result(group_id)
    }
}

#[cfg(test)]
#[path = "tests/child_stack_tests.rs"]
mod child_stack_tests;

#[cfg(test)]
#[path = "tests/child_stack_span_console_tests.rs"]
mod child_stack_span_console_tests;
