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

/// Offsets from the sys_rsx context base to the RAM-backed
/// substructures (`driver_info`, `reports`). The DMA control
/// registers live in the MMIO region at
/// [`control_register::DMA_CONTROL_BASE`] and are not allocated
/// from the rsx-region base.
pub mod region {
    /// Offset to the driver-info region.
    pub const DRIVER_INFO_OFFSET: u32 = 0x0010_0000;
    /// Offset to the reports region.
    pub const REPORTS_OFFSET: u32 = 0x0020_0000;
    /// Bytes reserved per sys_rsx context (covers driver_info,
    /// reports, plus padding).
    pub const CONTEXT_RESERVATION: u32 = 0x0030_0000;
}

/// `sys_rsx_device_map` (675) OUT-pointer value and the kernel
/// reservation it lives in. RPCS3 documents the address range
/// `0x40000000..0xB0000000` in its `sys_rsx.cpp` and allocates
/// inside it via `vm::reserve_map(vm::rsx_context, 0, 0x10000000,
/// 0x403)` -- the `0x10000000` size is the PS3 ABI reservation; the
/// address itself is a deterministic pick from the documented
/// range.
pub mod device_map {
    /// Device-map address `sys_rsx_device_map (675)` returns in
    /// its `dev_addr` OUT for `dev_id == 8`.
    pub const ADDR: u32 = 0x4000_0000;

    /// Size of the kernel `rsx_context` reservation that holds
    /// [`ADDR`]. The host's mmapper allocator skips
    /// `[ADDR, ADDR + RESERVATION_SIZE)` so device-map and mmapper
    /// allocations never alias.
    pub const RESERVATION_SIZE: u32 = 0x1000_0000;
}

/// `sys_rsx_context_iomap` (672) argument-validation constants per
/// RPCS3's `sys_rsx.cpp` IO-map handler.
pub mod iomap {
    /// `context_id` value the kernel pins for the single allocated
    /// RSX context.
    pub const CONTEXT_ID: u32 = 0x5555_5555;

    /// 1 MiB alignment mask. `io`, `ea`, and `size` must all be
    /// 1 MiB aligned; non-zero `value & ALIGN_MASK` is `CELL_EINVAL`.
    pub const ALIGN_MASK: u32 = 0x000F_FFFF;
}

/// Fixed-address RSX command-FIFO control register slots inside the
/// MMIO region. The guest reads / writes these to drive the RSX
/// FIFO; both real PS3 and CellGov surface them at the same absolute
/// addresses. Libgcm receives [`control_register::DMA_CONTROL_BASE`]
/// from `sys_rsx_context_allocate` (670) as the dma_control OUT and
/// adds `+0x40` internally to derive the put-pointer write target --
/// see RPCS3's `cellGcmSys.cpp` for the same derivation.
pub mod control_register {
    /// Guest address of the RSX dma_control region base.
    /// `sys_rsx_context_allocate` (670) returns this in its
    /// `lpar_dma_control` OUT.
    pub const DMA_CONTROL_BASE: u32 = 0xC000_0000;

    /// Guest address of the RSX control register's `put` slot
    /// (`DMA_CONTROL_BASE + 0x40`).
    pub const PUT_ADDR: u32 = 0xC000_0040;

    /// Guest address of the RSX control register's `get` slot.
    pub const GET_ADDR: u32 = 0xC000_0044;

    /// Guest address of the RSX control register's `reference` slot.
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
