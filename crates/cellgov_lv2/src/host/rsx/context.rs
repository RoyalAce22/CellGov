//! `sys_rsx_context_allocate` (670) and `sys_rsx_context_free` (671).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_rsx::{
    control_register, driver_info, driver_info_init, event_queue, region, reports,
};

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

use super::init::{write_rsx_driver_info_init, write_rsx_reports_init};
use super::state::{SysRsxContext, RSX_CONTEXT_ID};

impl Lv2Host {
    /// `sys_rsx_context_allocate` (670): reserve driver-info / reports region,
    /// create the handler event-queue/port pair, and write the fixed MMIO
    /// dma_control base into `lpar_dma_control` (libgcm derives PUT_ADDR =
    /// base + 0x40).
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` on double-allocate (single-context invariant);
    /// `CELL_ENOMEM` if the reservation does not fit in the remaining region.
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
    ) -> Lv2Dispatch {
        if self.rsx_context.allocated {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let base = if self.rsx_context.pending_mem_addr != 0 {
            self.rsx_context.pending_mem_addr
        } else {
            let Some(end) = self
                .rsx_mem_alloc_ptr
                .checked_add(region::CONTEXT_RESERVATION)
            else {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
            };
            if end > Self::SYS_RSX_MEM_END {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
            }
            let start = self.rsx_mem_alloc_ptr;
            self.rsx_mem_alloc_ptr = end;
            start
        };
        let dma_control_addr = control_register::DMA_CONTROL_BASE;
        let driver_info_addr = base + region::DRIVER_INFO_OFFSET;
        let reports_addr = base + region::REPORTS_OFFSET;

        // port_id == queue_id: single kernel id for the 1:1 port/queue
        // binding driver_info.handler_queue exposes.
        let queue_id = self.alloc_id();
        let queue_created = self
            .event_queues
            .create_with_id(queue_id, event_queue::SIZE);
        debug_assert!(
            queue_created,
            "sys_rsx event queue id {queue_id:#x} collided with existing queue"
        );

        self.rsx_context = SysRsxContext {
            allocated: true,
            context_id: RSX_CONTEXT_ID,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id: queue_id,
            event_port_id: queue_id,
            mem_ctx,
            system_mode,
            pending_mem_addr: self.rsx_context.pending_mem_addr,
            ..SysRsxContext::new()
        };

        let mk_write_u32 = |ptr: u32, value: u32| Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(ptr, 4),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let mk_write_u64 = |ptr: u32, value: u32| Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(ptr, 8),
            bytes: WritePayload::from_slice(&(value as u64).to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        let mut reports_bytes = vec![0u8; reports::SIZE];
        write_rsx_reports_init(&mut reports_bytes);
        let reports_init = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(reports_addr, reports::SIZE as u32),
            bytes: WritePayload::from_slice(&reports_bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

        let mut driver_info_bytes = vec![0u8; driver_info::SIZE];
        write_rsx_driver_info_init(
            &mut driver_info_bytes,
            driver_info_init::MEMORY_SIZE,
            system_mode as u32,
            queue_id,
        );
        let driver_info_init_effect = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(driver_info_addr, driver_info::SIZE as u32),
            bytes: WritePayload::from_slice(&driver_info_bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };

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
    pub(in crate::host) fn dispatch_sys_rsx_context_free_noop(&mut self) -> Lv2Dispatch {
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
