//! `sys_rsx_context_allocate` (670) and `sys_rsx_context_free` (671).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;
use cellgov_ps3_abi::sys_rsx::{driver_info, driver_info_init, event_queue, region, reports};

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

use super::init::{write_rsx_driver_info_init, write_rsx_reports_init};
use super::state::{SysRsxContext, RSX_CONTEXT_ID};

impl Lv2Host {
    /// sys_rsx_context_allocate (670). Reserves a 0x300000-byte slice,
    /// splits it into DMA-control / driver-info / reports sub-regions,
    /// emits init effects for reports and driver-info, and creates the
    /// handler event-queue / port pair.
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
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let base = if self.rsx_context.pending_mem_addr != 0 {
            self.rsx_context.pending_mem_addr
        } else {
            let Some(end) = self
                .rsx_mem_alloc_ptr
                .checked_add(region::CONTEXT_RESERVATION)
            else {
                return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
            };
            if end > Self::SYS_RSX_MEM_END {
                return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
            }
            let start = self.rsx_mem_alloc_ptr;
            self.rsx_mem_alloc_ptr = end;
            start
        };
        let dma_control_addr = base + region::DMA_CONTROL_OFFSET;
        let driver_info_addr = base + region::DRIVER_INFO_OFFSET;
        let reports_addr = base + region::REPORTS_OFFSET;

        // port_id == queue_id: the event model uses a single kernel id
        // for the 1:1 port/queue binding driver_info.handler_queue exposes.
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

    /// sys_rsx_context_free (671). No-op: the single-context model
    /// does not tear down state, and a subsequent allocate is still
    /// rejected. Logs an invariant-break so a caller that frees and
    /// then re-allocates expecting a fresh context will be visible
    /// in the trace; until that case is observed in the title
    /// corpus, the no-op-with-trace is treated as a convergent
    /// honest gap.
    pub(in crate::host) fn dispatch_sys_rsx_context_free_noop(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.sys_rsx_context_free_noop",
            format_args!("sys_rsx_context_free is a no-op in the single-context model"),
        );
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::rsx::test_helpers::{context_allocate_request, extract_write_u64};
    use crate::host::test_support::{extract_write_u32, FakeRuntime};
    use crate::request::Lv2Request;

    #[test]
    fn sys_rsx_context_allocate_writes_four_out_pointers_and_reports_init() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert_eq!(effects.len(), 6);
        assert_eq!(extract_write_u32(&effects[0]), RSX_CONTEXT_ID);
        assert_eq!(
            extract_write_u64(&effects[1]),
            Lv2Host::SYS_RSX_MEM_BASE as u64
        );
        assert_eq!(
            extract_write_u64(&effects[2]),
            (Lv2Host::SYS_RSX_MEM_BASE + region::DRIVER_INFO_OFFSET) as u64
        );
        assert_eq!(
            extract_write_u64(&effects[3]),
            (Lv2Host::SYS_RSX_MEM_BASE + region::REPORTS_OFFSET) as u64
        );

        let Effect::SharedWriteIntent { range, bytes, .. } = &effects[4] else {
            panic!("expected SharedWriteIntent for reports init");
        };
        assert_eq!(
            range.start().raw(),
            (Lv2Host::SYS_RSX_MEM_BASE + region::REPORTS_OFFSET) as u64
        );
        let b = bytes.bytes();
        assert_eq!(b.len(), reports::SIZE);
        let sentinel = u32::from_be_bytes([b[0xFF0], b[0xFF1], b[0xFF2], b[0xFF3]]);
        assert_eq!(sentinel, 0x1337_C0D3);
        assert_eq!(&b[0x1000..0x1008], &[0xFF; 8]);
        assert_eq!(&b[0x140C..0x1410], &[0xFF; 4]);

        let Effect::SharedWriteIntent { range, bytes, .. } = &effects[5] else {
            panic!("expected SharedWriteIntent for driver-info init");
        };
        assert_eq!(
            range.start().raw(),
            (Lv2Host::SYS_RSX_MEM_BASE + region::DRIVER_INFO_OFFSET) as u64
        );
        let b = bytes.bytes();
        assert_eq!(b.len(), driver_info::SIZE);
        assert_eq!(
            u32::from_be_bytes([b[0x00], b[0x01], b[0x02], b[0x03]]),
            driver_info_init::VERSION_DRIVER
        );
        assert_eq!(
            u32::from_be_bytes([b[0x04], b[0x05], b[0x06], b[0x07]]),
            driver_info_init::VERSION_GPU
        );
        assert_eq!(
            u32::from_be_bytes([b[0x0C], b[0x0D], b[0x0E], b[0x0F]]),
            driver_info_init::HARDWARE_CHANNEL
        );
        assert_eq!(
            u32::from_be_bytes([b[0x10], b[0x11], b[0x12], b[0x13]]),
            driver_info_init::NVCORE_FREQUENCY
        );
        assert_eq!(
            u32::from_be_bytes([b[0x2C], b[0x2D], b[0x2E], b[0x2F]]),
            driver_info_init::REPORTS_NOTIFY_OFFSET
        );
        assert_eq!(
            u32::from_be_bytes([b[0x34], b[0x35], b[0x36], b[0x37]]),
            driver_info_init::REPORTS_REPORT_OFFSET
        );

        let ctx = host.sys_rsx_context();
        assert!(ctx.allocated);
        assert_eq!(ctx.context_id, RSX_CONTEXT_ID);
        assert_eq!(ctx.dma_control_addr, Lv2Host::SYS_RSX_MEM_BASE);
        assert_eq!(
            ctx.driver_info_addr,
            Lv2Host::SYS_RSX_MEM_BASE + region::DRIVER_INFO_OFFSET
        );
        assert_eq!(
            ctx.reports_addr,
            Lv2Host::SYS_RSX_MEM_BASE + region::REPORTS_OFFSET
        );
        assert_eq!(ctx.mem_ctx, 0xA001);
    }

    #[test]
    fn sys_rsx_context_allocate_second_call_rejects_with_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let _ = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let d = host.dispatch(
            context_allocate_request(0x2000, 0x2008, 0x2010, 0x2018, 0xA001),
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, effects } if code == u64::from(errno::CELL_EINVAL) && effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_free_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextFree {
                context_id: RSX_CONTEXT_ID,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_allocate_registers_event_queue_in_handler_queue() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let source = UnitId::new(0);

        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        let Effect::SharedWriteIntent { bytes, .. } = &effects[5] else {
            panic!("expected SharedWriteIntent for driver-info init");
        };
        let b = bytes.bytes();
        let queue_id = u32::from_be_bytes([
            b[driver_info::HANDLER_QUEUE_OFFSET],
            b[driver_info::HANDLER_QUEUE_OFFSET + 1],
            b[driver_info::HANDLER_QUEUE_OFFSET + 2],
            b[driver_info::HANDLER_QUEUE_OFFSET + 3],
        ]);
        assert_ne!(queue_id, 0);
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.event_queue_id, queue_id);
        assert_eq!(ctx.event_port_id, queue_id);
    }
}
