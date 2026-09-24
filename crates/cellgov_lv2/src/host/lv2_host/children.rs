//! Child-thread stacks and SPU group notes.

use crate::ppu_thread::ThreadStack;

use super::model::Lv2Host;

impl Lv2Host {
    /// Allocate a child-thread stack of `size` bytes at `align`.
    pub fn allocate_child_stack(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        self.state.stack_allocator.allocate(size, align)
    }

    /// Return the stack block a refused thread-create allocated.
    ///
    /// # Cross-module contract
    ///
    /// Must be called before anything else can allocate: the arena
    /// only rewinds its most recent block. A mismatch is logged and
    /// the block leaks rather than corrupting live stacks.
    pub fn free_child_stack(&mut self, base: u64, size: u64) {
        // `ThreadStack::new` asserts the minimum-frame floor and
        // the arena never hands out a block below it, so a sub-floor
        // size is the same "not the arena's block" refusal -- report
        // it rather than aborting the run inside the assert.
        let freed = size >= 0x10
            && self
                .state
                .stack_allocator
                .free_last(&ThreadStack::new(base, size));
        if !freed {
            self.log_invariant_break(
                "ppu_thread_create.stack_free_mismatch",
                format_args!(
                    "free_child_stack(0x{base:x}, 0x{size:x}) is not the arena's most \
                     recent block; refusal-path free out of order, block leaked"
                ),
            );
        }
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
}
