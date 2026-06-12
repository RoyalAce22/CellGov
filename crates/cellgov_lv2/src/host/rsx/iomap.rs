//! `sys_rsx_context_iomap` (672) dispatch.

use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::process_address_space::{PS3_RSX_BASE, PS3_RSX_IOMAP_SIZE};
use cellgov_ps3_abi::sys_rsx::iomap;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// Record the io -> ea mapping on the live `SysRsxContext`.
    ///
    /// # Cross-module contract
    ///
    /// Only the most recent mapping is retained; a second call overwrites
    /// the first. A single triple covers every modeled title (each issues
    /// one window through `_cellGcmInitBody`).
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` for: `context_id != 0x5555_5555`, `size == 0`, any of
    /// `io`/`ea`/`size` not 1 MiB-aligned, `ea + size` crossing into
    /// [`PS3_RSX_BASE`], or `io + size` exceeding the baked iomap region.
    /// Only the io-over-cap path logs
    /// `dispatch.sys_rsx_context_iomap_oversized`.
    pub(in crate::host) fn dispatch_sys_rsx_context_iomap(
        &mut self,
        context_id: u32,
        io: u32,
        ea: u32,
        size: u32,
        _flags: u64,
    ) -> Lv2Dispatch {
        if context_id != iomap::CONTEXT_ID {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if size == 0
            || (io & iomap::ALIGN_MASK) != 0
            || (ea & iomap::ALIGN_MASK) != 0
            || (size & iomap::ALIGN_MASK) != 0
        {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // u64 catches u32 wrap; PS3_RSX_BASE is RPCS3's local_mem_base.
        if u64::from(ea) + u64::from(size) > PS3_RSX_BASE {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // u64 catches u32 wrap (e.g. io=0xFFF0_0000+size=0x10_0000
        // wraps to 0).
        const BAKED_IOMAP_SIZE: u64 = PS3_RSX_IOMAP_SIZE as u64;
        if u64::from(io) + u64::from(size) > BAKED_IOMAP_SIZE {
            self.log_invariant_break(
                "dispatch.sys_rsx_context_iomap_oversized",
                format_args!(
                    "sys_rsx_context_iomap io={io:#x}+size={size:#x} exceeds baked \
                     region {BAKED_IOMAP_SIZE:#x}; returning CELL_EINVAL"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        self.rsx_context.iomap_io = io;
        self.rsx_context.iomap_ea = ea;
        self.rsx_context.iomap_size = size;
        Lv2Dispatch::immediate(cell_errors::CELL_OK.into())
    }
}

#[cfg(test)]
#[path = "tests/iomap_tests.rs"]
mod tests;
