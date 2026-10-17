//! `sys_rsx_context_allocate` (670) and `sys_rsx_context_free` (671).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::process::SYS_EVENT_QUEUE_OBJECT;
use cellgov_ps3_abi::lv2::rsx::{driver_info, driver_info_init, event_queue, region, reports};

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

use super::init::{write_rsx_driver_info_init, write_rsx_reports_init};
use super::state::{SysRsxContext, RSX_CONTEXT_ID};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_rsx_context_allocate` (670): open the context window, create
    /// the handler event-queue/port pair, and hand the guest the window's
    /// three blocks (libgcm adds `control_register::PUT_OFFSET` to
    /// `dma_control` for the put slot).
    ///
    /// The window opens `region::WINDOW_OFFSET` into the lowest area
    /// granule the caller's process holds nothing in, as the
    /// `lv2_rsx_answers` and `lv2_rsx_window_held` captures read it:
    /// the DMA control block at its base, the driver-info block one
    /// megabyte above it and the reports block two. The runtime maps the
    /// window in the caller's space before the batch commits, so the
    /// reports and driver-info init fills go out as effects in the same
    /// batch as the four out-pointer writes.
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` on double-allocate (single-context invariant);
    /// `CELL_ENOMEM` when no area granule is free, which no capture
    /// shows; `CELL_EAGAIN` when the kernel-id table has no id for the
    /// queue.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::host) fn dispatch_sys_rsx_context_allocate(
        &mut self,
        context_id_ptr: u32,
        lpar_dma_control_ptr: u32,
        lpar_driver_info_ptr: u32,
        lpar_reports_ptr: u32,
        mem_ctx: u64,
        system_mode: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if self.state.rsx_context.allocated {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let granule = cellgov_ps3_abi::lv2::memory::VM_AREA_GRANULE as u32;
        let pid = self.state.processes.process_of_unit(requester);
        let Some(granule_base) = self.free_area_base(pid, granule) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        // port_id == queue_id: single kernel id for the 1:1 port/queue
        // binding driver_info.handler_queue exposes.
        let Some(queue_id) = self.alloc_id(requester, SYS_EVENT_QUEUE_OBJECT) else {
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
        };
        let dma_control_addr = granule_base + region::WINDOW_OFFSET;
        let driver_info_addr = dma_control_addr + region::DRIVER_INFO_OFFSET;
        let reports_addr = dma_control_addr + region::REPORTS_OFFSET;
        self.derived.pending_window_install = Some(dma_control_addr);

        let queue_created = self
            .state
            .event_queues
            .create_with_id(queue_id, event_queue::SIZE);
        debug_assert!(
            queue_created,
            "sys_rsx event queue id {queue_id:#x} collided with existing queue"
        );

        self.state.rsx_context = SysRsxContext {
            allocated: true,
            context_id: RSX_CONTEXT_ID,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id: queue_id,
            event_port_id: queue_id,
            mem_ctx,
            system_mode,
            pending_mem_addr: self.state.rsx_context.pending_mem_addr,
            ..SysRsxContext::new()
        };

        let mk_write_u32 = |ptr: u32, value: u32| {
            Effect::shared_write(
                ByteRange::contiguous_u32(ptr, 4),
                WritePayload::from_slice(&value.to_be_bytes()),
                requester,
                tick,
            )
        };
        let mk_write_u64 = |ptr: u32, value: u32| {
            Effect::shared_write(
                ByteRange::contiguous_u32(ptr, 8),
                WritePayload::from_slice(&(value as u64).to_be_bytes()),
                requester,
                tick,
            )
        };

        let mut reports_bytes = vec![0u8; reports::SIZE];
        write_rsx_reports_init(&mut reports_bytes);
        let reports_init = Effect::shared_write(
            ByteRange::contiguous_u32(reports_addr, reports::SIZE as u32),
            WritePayload::from_slice(&reports_bytes),
            requester,
            tick,
        );

        let mut driver_info_bytes = vec![0u8; driver_info::SIZE];
        write_rsx_driver_info_init(
            &mut driver_info_bytes,
            driver_info_init::MEMORY_SIZE,
            system_mode as u32,
            queue_id,
        );
        let driver_info_init_effect = Effect::shared_write(
            ByteRange::contiguous_u32(driver_info_addr, driver_info::SIZE as u32),
            WritePayload::from_slice(&driver_info_bytes),
            requester,
            tick,
        );

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![
                mk_write_u32(context_id_ptr, RSX_CONTEXT_ID),
                mk_write_u64(lpar_dma_control_ptr, dma_control_addr),
                mk_write_u64(lpar_driver_info_ptr, driver_info_addr),
                mk_write_u64(lpar_reports_ptr, reports_addr),
                reports_init,
                driver_info_init_effect,
            ],
        }
    }

    /// `sys_rsx_context_free` (671): no-op in the single-context model;
    /// logs an invariant break so a free-then-realloc caller is traceable.
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` for an id that names no allocated context, as the
    /// `lv2_rsx_answers` capture reads for `0xDEAD`.
    pub(in crate::host) fn dispatch_sys_rsx_context_free(
        &mut self,
        context_id: u32,
    ) -> Lv2Dispatch {
        if !self.state.rsx_context.allocated || context_id != self.state.rsx_context.context_id {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        self.log_invariant_break(
            "dispatch.sys_rsx_context_free_noop",
            format_args!("sys_rsx_context_free is a no-op in the single-context model"),
        );
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/context_tests.rs"]
mod tests;
