//! `sys_rsx_memory_allocate` (665) and `sys_rsx_memory_free` (667).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_rsx_memory_allocate` (665): bump-allocate `size` bytes and write
    /// `mem_handle` (u32 BE) and `mem_addr` (u64 BE) to the OUT pointers.
    ///
    /// # Errors
    ///
    /// `CELL_ENOMEM` if `size == 0`, the cursor would wrap, or the end
    /// address exceeds [`Lv2Host::SYS_RSX_MEM_END`].
    pub(in crate::host) fn dispatch_sys_rsx_memory_allocate(
        &mut self,
        mem_handle_ptr: u32,
        mem_addr_ptr: u32,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if size == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        let Some(end) = self.rsx_mem_alloc_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        if end > Self::SYS_RSX_MEM_END {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }

        let handle = self.rsx_mem_handle_counter;
        let addr = self.rsx_mem_alloc_ptr;
        self.rsx_mem_alloc_ptr = end;
        self.rsx_mem_handle_counter = handle.wrapping_add(1);
        // Subsequent sys_rsx_context_allocate consumes this reservation
        // instead of bumping the cursor again.
        self.rsx_context.pending_mem_addr = addr;

        let handle_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_handle_ptr, 4),
            bytes: WritePayload::from_slice(&handle.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let addr_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_addr_ptr, 8),
            bytes: WritePayload::from_slice(&(addr as u64).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![handle_write, addr_write],
        }
    }

    /// `sys_rsx_memory_free` (669): no-op against the bump allocator; logs an
    /// invariant break so a free-then-realloc caller is traceable.
    pub(in crate::host) fn dispatch_sys_rsx_memory_free_noop(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.sys_rsx_memory_free_noop",
            format_args!("sys_rsx_memory_free is a no-op against the bump allocator"),
        );
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
