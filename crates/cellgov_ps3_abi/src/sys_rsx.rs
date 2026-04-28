//! sys_rsx PS3 ABI: region offsets, struct sizes, driver-info init
//! values, package ids, display-buffer limit.
//!
//! Behaviour (the syscall handlers, the `RsxReports` / `RsxDriverInfo`
//! struct definitions, the `SysRsxContext` state machine, the init
//! sentinels CellGov picks) lives in `cellgov_lv2::host::rsx` and
//! `cellgov_core::rsx`; this module is data only.
//!
//! CellGov-internal sentinels (`RSX_CONTEXT_ID = 0x5555_5555`,
//! `SEMAPHORE_INIT_PATTERN`, `PACKAGE_CELLGOV_*`) are NOT PS3 ABI and
//! stay in `cellgov_lv2::host::rsx`.

/// Offsets from the sys_rsx context base to each substructure.
pub mod region {
    /// Offset to the DMA control region.
    pub const DMA_CONTROL_OFFSET: u32 = 0x0000_0000;
    /// Offset to the driver-info region.
    pub const DRIVER_INFO_OFFSET: u32 = 0x0010_0000;
    /// Offset to the reports region.
    pub const REPORTS_OFFSET: u32 = 0x0020_0000;
    /// Bytes reserved per sys_rsx context (covers all three regions
    /// plus padding).
    pub const CONTEXT_RESERVATION: u32 = 0x0030_0000;
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

/// Values `sys_rsx_context_allocate` stamps into the driver-info
/// region during init.
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
    /// Queue depth.
    pub const SIZE: u32 = 0x20;
}

/// `sys_rsx_context_attribute` package ids (the `package_id` argument).
pub mod package {
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
