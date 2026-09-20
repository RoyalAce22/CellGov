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
/// pick from the range the asserts below bound.
pub mod device_map {
    /// `dev_addr` OUT for `dev_id == 8`.
    pub const ADDR: u32 = 0x4000_0000;

    /// Size of the kernel `rsx_context` reservation holding [`ADDR`]; the
    /// mmapper allocator skips `[ADDR, ADDR + RESERVATION_SIZE)` to avoid alias.
    pub const RESERVATION_SIZE: u32 = 0x1000_0000;

    // ADDR sits at or above the IO window base a title's own
    // sys_rsx_context_iomap call asks for, and clear of the control
    // region, whose slots a guest reaches by fixed offsets.
    const _: () = assert!(ADDR as u64 >= crate::hw::address_space::PS3_RSX_IOMAP_BASE);
    const _: () =
        assert!(ADDR.saturating_add(RESERVATION_SIZE) <= super::control_register::DMA_CONTROL_BASE);
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
///
/// The semaphore block runs from offset 0 to
/// [`driver_info_init::REPORTS_NOTIFY_OFFSET`]. The notify array and
/// the report array follow at the two offsets `driver_info` publishes
/// to the guest. Those two offsets and [`reports::SIZE`] fix each
/// array's byte span, and [`reports::ENTRY_SIZE`] fixes its count.
/// What the kernel leaves in the semaphore block is unestablished.
pub mod reports {
    /// `sizeof(RsxReports)`.
    pub const SIZE: usize = 0x9400;
    /// `sizeof(RsxNotify)`, and `sizeof(RsxReport)`: both are a
    /// 64-bit timestamp followed by 8 bytes of payload.
    pub const ENTRY_SIZE: usize = 16;
    /// Notify entries in the array at
    /// [`super::driver_info_init::REPORTS_NOTIFY_OFFSET`].
    pub const NOTIFY_COUNT: usize = 64;
    /// Report entries in the array at
    /// [`super::driver_info_init::REPORTS_REPORT_OFFSET`].
    pub const REPORT_COUNT: usize = 2048;
    /// Bytes of an entry the leading timestamp occupies.
    pub const TIMESTAMP_SIZE: usize = 8;
    /// Offset of a report entry's trailing pad word.
    pub const REPORT_PAD_OFFSET: usize = 12;
    /// Width of that pad word.
    pub const REPORT_PAD_SIZE: usize = 4;

    // A wrong count or stride fails the build rather than shifting
    // the arrays inside the region.
    const _: () = assert!(
        super::driver_info_init::REPORTS_NOTIFY_OFFSET as usize + NOTIFY_COUNT * ENTRY_SIZE
            == super::driver_info_init::REPORTS_REPORT_OFFSET as usize
    );
    const _: () = assert!(
        super::driver_info_init::REPORTS_REPORT_OFFSET as usize + REPORT_COUNT * ENTRY_SIZE == SIZE
    );
    const _: () = assert!(REPORT_PAD_OFFSET + REPORT_PAD_SIZE == ENTRY_SIZE);
    const _: () = assert!(TIMESTAMP_SIZE <= REPORT_PAD_OFFSET);
}

/// `RsxDriverInfo` substructure.
///
/// `sys_rsx_context_allocate` (670) writes every field below as a
/// 32-bit big-endian word. The offsets are unestablished: no trace in
/// a capture records a guest read of this region. The field order
/// alone pins each word to its offset. A libgcm read of the
/// driver-info region would witness them. The words the kernel leaves
/// zero carry no constant here.
pub mod driver_info {
    /// `sizeof(RsxDriverInfo)`.
    pub const SIZE: usize = 0x12F8;
    /// `version_driver`.
    pub const VERSION_DRIVER_OFFSET: usize = 0x00;
    /// `version_gpu`.
    pub const VERSION_GPU_OFFSET: usize = 0x04;
    /// `memory_size` -- local RSX memory exposed to the caller.
    pub const MEMORY_SIZE_OFFSET: usize = 0x08;
    /// `hardware_channel`.
    pub const HARDWARE_CHANNEL_OFFSET: usize = 0x0C;
    /// `nvcore_frequency`.
    pub const NVCORE_FREQUENCY_OFFSET: usize = 0x10;
    /// `memory_frequency`.
    pub const MEMORY_FREQUENCY_OFFSET: usize = 0x14;
    /// `reports_notify_offset` -- notify array, relative to the
    /// reports region base.
    pub const REPORTS_NOTIFY_OFFSET_FIELD: usize = 0x2C;
    /// `reports_offset` -- semaphore block, same base.
    pub const REPORTS_OFFSET_FIELD: usize = 0x30;
    /// `reports_report_offset` -- report array, same base.
    pub const REPORTS_REPORT_OFFSET_FIELD: usize = 0x34;
    /// `system_mode`.
    pub const SYSTEM_MODE_OFFSET: usize = 0x50;
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
