//! Per-context bookkeeping for `sys_rsx`: [`SysRsxContext`], its per-slot
//! [`RsxDisplayBuffer`] payload, and the [`RSX_CONTEXT_ID`] sentinel.

use cellgov_ps3_abi::lv2::rsx::display_buffer;

/// Fixed `context_id` returned from `sys_rsx_context_allocate`. CellGov
/// sentinel; the PS3 ABI does not pin a value.
pub const RSX_CONTEXT_ID: u32 = 0x5555_5555;

/// Per-slot display buffer metadata decoded from a `SET_DISPLAY_BUFFER` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RsxDisplayBuffer {
    /// Guest-memory offset of the buffer.
    pub offset: u32,
    /// Row pitch in bytes.
    pub pitch: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Per-context bookkeeping. Single-instance per [`crate::host::Lv2Host`];
/// multi-context support would require keying by `context_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysRsxContext {
    /// True once `sys_rsx_context_allocate` has fired.
    pub allocated: bool,
    /// Context id returned to the guest ([`RSX_CONTEXT_ID`] when set).
    pub context_id: u32,
    /// DMA control region guest address.
    pub dma_control_addr: u32,
    /// Driver-info region guest address.
    pub driver_info_addr: u32,
    /// Reports region guest address.
    pub reports_addr: u32,
    /// Event-queue handle.
    pub event_queue_id: u32,
    /// Event-port handle.
    pub event_port_id: u32,
    /// Memory handle the context was allocated against.
    pub mem_ctx: u64,
    /// System-mode flag word passed in by the caller.
    pub system_mode: u64,
    /// Base reserved by the most recent `sys_rsx_memory_allocate`, reused
    /// by `sys_rsx_context_allocate`; zero means memory_allocate has not fired.
    pub pending_mem_addr: u32,
    /// Flip-handler callback OPD address (0 = unregistered).
    pub flip_handler_addr: u32,
    /// Vblank-handler callback OPD address (0 = unregistered).
    pub vblank_handler_addr: u32,
    /// User-handler callback OPD address (0 = unregistered).
    pub user_handler_addr: u32,
    /// Display-buffer metadata. Slots `0..display_buffers_count` are populated.
    pub display_buffers: [RsxDisplayBuffer; display_buffer::COUNT_MAX],
    /// Count of populated display-buffer slots (monotonic).
    pub display_buffers_count: u32,
    /// Flip mode flag (1 = hsync, 2 = vsync). 0 until FLIP_MODE fires.
    pub flip_mode: u32,
    /// Most recent `sys_rsx_context_iomap` mapping; `iomap_size == 0` means
    /// no mapping has been recorded.
    pub iomap_io: u32,
    /// EA the most recent iomap call mapped to (see [`Self::iomap_io`]).
    pub iomap_ea: u32,
    /// Size in bytes of the most recent iomap mapping; see [`Self::iomap_io`].
    pub iomap_size: u32,
    /// FIFO GET pointer; zero until `sys_rsx_context_attribute(FIFO_SETUP)` fires.
    pub fifo_get: u32,
    /// FIFO PUT pointer; zero until `sys_rsx_context_attribute(FIFO_SETUP)` fires.
    pub fifo_put: u32,
}

impl SysRsxContext {
    /// Zero-initialized context (`allocated == false`).
    #[inline]
    pub const fn new() -> Self {
        Self {
            allocated: false,
            context_id: 0,
            dma_control_addr: 0,
            driver_info_addr: 0,
            reports_addr: 0,
            event_queue_id: 0,
            event_port_id: 0,
            mem_ctx: 0,
            system_mode: 0,
            pending_mem_addr: 0,
            flip_handler_addr: 0,
            vblank_handler_addr: 0,
            user_handler_addr: 0,
            display_buffers: [RsxDisplayBuffer {
                offset: 0,
                pitch: 0,
                width: 0,
                height: 0,
            }; display_buffer::COUNT_MAX],
            display_buffers_count: 0,
            flip_mode: 0,
            iomap_io: 0,
            iomap_ea: 0,
            iomap_size: 0,
            fifo_get: 0,
            fifo_put: 0,
        }
    }
}

/// Lane layout of the RSX context:
///
/// - Fields 1 to 20 are the scalar fields, one lane each.
/// - Fields 21 to 24 are the offset, pitch, width and height of display
///   buffer `i`, at slot `i`.
///
/// The exhaustive destructure makes a new field without a lane a
/// compile error.
impl cellgov_mem::lanes::LaneValue for SysRsxContext {
    fn lanes(&self, lanes: &mut cellgov_mem::lanes::ObjectLanes) {
        let Self {
            allocated,
            context_id,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id,
            event_port_id,
            mem_ctx,
            system_mode,
            pending_mem_addr,
            flip_handler_addr,
            vblank_handler_addr,
            user_handler_addr,
            display_buffers,
            display_buffers_count,
            flip_mode,
            iomap_io,
            iomap_ea,
            iomap_size,
            fifo_get,
            fifo_put,
        } = *self;
        let scalars = [
            u64::from(allocated),
            u64::from(context_id),
            u64::from(dma_control_addr),
            u64::from(driver_info_addr),
            u64::from(reports_addr),
            u64::from(event_queue_id),
            u64::from(event_port_id),
            mem_ctx,
            system_mode,
            u64::from(pending_mem_addr),
            u64::from(flip_handler_addr),
            u64::from(vblank_handler_addr),
            u64::from(user_handler_addr),
            u64::from(display_buffers_count),
            u64::from(flip_mode),
            u64::from(iomap_io),
            u64::from(iomap_ea),
            u64::from(iomap_size),
            u64::from(fifo_get),
            u64::from(fifo_put),
        ];
        for (field, value) in (1..).zip(scalars) {
            lanes.lane(field, 0, value);
        }
        for (slot, buf) in display_buffers.iter().enumerate() {
            let slot = slot as u64;
            lanes.lane(21, slot, u64::from(buf.offset));
            lanes.lane(22, slot, u64::from(buf.pitch));
            lanes.lane(23, slot, u64::from(buf.width));
            lanes.lane(24, slot, u64::from(buf.height));
        }
    }
}

impl Default for SysRsxContext {
    fn default() -> Self {
        Self::new()
    }
}
