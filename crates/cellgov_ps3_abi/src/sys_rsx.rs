//! sys_rsx PS3 ABI constants: region offsets, struct sizes, driver-info init
//! values, package ids, display-buffer limit. Data only -- behaviour lives in
//! `cellgov_lv2::host::rsx` and `cellgov_core::rsx`.

/// Offsets from the sys_rsx context base to the RAM-backed substructures.
/// DMA control registers live separately in MMIO at
/// [`control_register::DMA_CONTROL_BASE`].
pub mod region {
    /// Driver-info region offset from the context base.
    pub const DRIVER_INFO_OFFSET: u32 = 0x0010_0000;
    /// Reports region offset from the context base.
    pub const REPORTS_OFFSET: u32 = 0x0020_0000;
    /// Bytes reserved per sys_rsx context (covers driver_info, reports, padding).
    pub const CONTEXT_RESERVATION: u32 = 0x0030_0000;
}

/// `sys_rsx_device_map` (675) OUT-pointer value and its kernel reservation.
/// `RESERVATION_SIZE` is the PS3 ABI reservation; `ADDR` is a deterministic
/// pick from the documented `0x40000000..0xB0000000` range.
pub mod device_map {
    /// `dev_addr` OUT for `dev_id == 8`.
    pub const ADDR: u32 = 0x4000_0000;

    /// Size of the kernel `rsx_context` reservation holding [`ADDR`]; the
    /// mmapper allocator skips `[ADDR, ADDR + RESERVATION_SIZE)` to avoid alias.
    pub const RESERVATION_SIZE: u32 = 0x1000_0000;
}

/// `sys_rsx_context_iomap` (672) argument-validation constants.
pub mod iomap {
    /// `context_id` the kernel pins for the single allocated RSX context.
    pub const CONTEXT_ID: u32 = 0x5555_5555;

    /// 1 MiB alignment mask. `io`, `ea`, and `size` must all be 1 MiB
    /// aligned; non-zero `value & ALIGN_MASK` is `CELL_EINVAL`.
    pub const ALIGN_MASK: u32 = 0x000F_FFFF;
}

/// Fixed-address RSX command-FIFO control register slots in MMIO. Libgcm
/// receives [`control_register::DMA_CONTROL_BASE`] from
/// `sys_rsx_context_allocate` (670) and derives the put/get/ref slots by
/// adding `+0x40 / +0x44 / +0x48`.
pub mod control_register {
    /// RSX dma_control region base; `sys_rsx_context_allocate` (670) returns
    /// this in `lpar_dma_control`.
    pub const DMA_CONTROL_BASE: u32 = 0xC000_0000;

    /// RSX control register `put` slot (`DMA_CONTROL_BASE + 0x40`).
    pub const PUT_ADDR: u32 = 0xC000_0040;
    /// RSX control register `get` slot (`DMA_CONTROL_BASE + 0x44`).
    pub const GET_ADDR: u32 = 0xC000_0044;
    /// RSX control register `reference` slot (`DMA_CONTROL_BASE + 0x48`).
    pub const REF_ADDR: u32 = 0xC000_0048;
}

/// `RsxReports` substructure (1024 semaphore slots + 64 notify entries
/// + 2048 report entries).
pub mod reports {
    /// `sizeof(RsxReports)`.
    pub const SIZE: usize = 0x9400;
}

/// `RsxDriverInfo` substructure.
pub mod driver_info {
    /// `sizeof(RsxDriverInfo)`.
    pub const SIZE: usize = 0x12F8;
    /// Offset of the `handler_queue` field within `RsxDriverInfo`.
    pub const HANDLER_QUEUE_OFFSET: usize = 0x12D0;
}

/// Values `sys_rsx_context_allocate` stamps into the driver-info region
/// during init.
pub mod driver_info_init {
    /// Driver version word.
    pub const VERSION_DRIVER: u32 = 0x211;
    /// GPU version word.
    pub const VERSION_GPU: u32 = 0x5c;
    /// nvcore frequency in Hz.
    pub const NVCORE_FREQUENCY: u32 = 500_000_000;
    /// Memory frequency in Hz.
    pub const MEMORY_FREQUENCY: u32 = 650_000_000;
    /// Offset from reports_base to the notify array.
    pub const REPORTS_NOTIFY_OFFSET: u32 = 0x1000;
    /// Offset from reports_base to the semaphore block.
    pub const REPORTS_OFFSET_FIELD: u32 = 0;
    /// Offset from reports_base to the report entries.
    pub const REPORTS_REPORT_OFFSET: u32 = 0x1400;
    /// Hardware channel (games = 1, VSH = 0).
    pub const HARDWARE_CHANNEL: u32 = 1;
    /// Default local RSX memory exposed to games.
    pub const MEMORY_SIZE: u32 = 0x0F90_0000;
}

/// Default event-queue parameters for the RSX handler queue.
pub mod event_queue {
    /// RSX handler queue depth.
    pub const SIZE: u32 = 0x20;
}

/// `sys_rsx_context_attribute` package ids (the `package_id` argument).
pub mod package {
    /// FIFO setup. a3 = initial GET pointer, a4 = initial PUT pointer.
    pub const FIFO_SETUP: u32 = 0x001;
    /// Set flip mode (vsync / hsync).
    pub const FLIP_MODE: u32 = 0x101;
    /// Trigger a flip buffer.
    pub const FLIP_BUFFER: u32 = 0x102;
    /// Record display-buffer metadata.
    pub const SET_DISPLAY_BUFFER: u32 = 0x104;
}

/// `RsxDisplayBuffer` array sizing.
pub mod display_buffer {
    /// Maximum number of display buffer slots per context.
    pub const COUNT_MAX: usize = 8;
}
