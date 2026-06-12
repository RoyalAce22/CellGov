//! sys_rsx committed-state layouts.
//!
//! Mirrors RPCS3's `Emu/Cell/lv2/sys_rsx.h`: `RsxDmaControl`,
//! `RsxDriverInfo`, `RsxReports`. The Rust structs are layout
//! descriptors; the bytes live in guest memory and are covered by
//! the memory-state hash. Only [`RsxContext`]'s base addresses and
//! allocation flags fold into the sync-state hash.

use core::mem::size_of;

/// Hash-input shape version. Bump when the layout of
/// [`RsxContext::state_hash`] changes.
pub const STATE_HASH_FORMAT_VERSION: u8 = 2;

pub use cellgov_lv2::host::rsx::{RSX_CONTEXT_ID, SEMAPHORE_INIT_PATTERN};
pub use cellgov_ps3_abi::sys_rsx::{control_register, driver_info_init, region};

/// BE u32 semaphore slot.
pub type RsxSemaphore = u32;

/// Notify entry; 16-byte aligned.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxNotify {
    /// Notify timestamp.
    pub timestamp: u64,
    /// Reserved; RPCS3 writes zero.
    pub zero: u64,
}

/// Report entry; 16-byte aligned.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxReport {
    /// Report timestamp.
    pub timestamp: u64,
    /// Report value.
    pub val: u32,
    /// Padding.
    pub pad: u32,
}

/// Reports region: 1024 semaphore + 64 notify + 2048 report entries.
///
/// Label addressing strides 16 bytes: label `i` lands on
/// `semaphore[i * 4]` and overlaps the three trailing sentinels of
/// the init pattern until a guest NV method overwrites them. Label
/// 255 lands on semaphore index 1020, which LV2 init seeds with the
/// sentinel value.
#[repr(C)]
pub struct RsxReports {
    /// Semaphore slots.
    pub semaphore: [RsxSemaphore; 1024],
    /// Notify entries.
    pub notify: [RsxNotify; 64],
    /// Report entries.
    pub report: [RsxReport; 2048],
}

pub use cellgov_ps3_abi::rsx_nv_hardware::{LABEL_COUNT, LABEL_STRIDE};

/// DMA-control region. Put / get / ref live at +0x40 / +0x44 / +0x48;
/// total 0x58 bytes.
#[repr(C)]
pub struct RsxDmaControl {
    /// Reserved prefix.
    pub resv: [u8; 0x40],
    /// Put pointer.
    pub put: u32,
    /// Get pointer.
    pub get: u32,
    /// Reference value.
    pub ref_value: u32,
    /// Reserved.
    pub unk: [u32; 2],
    /// Reserved.
    pub unk1: u32,
}

/// Per-display head state; 0x40 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxDriverHead {
    /// Timestamp of the most recent flip.
    pub last_flip_time: u64,
    /// Flip / queue state flags.
    pub flip_flags: u32,
    /// Current flip offset.
    pub offset: u32,
    /// Currently-displayed buffer id.
    pub flip_buffer_id: u32,
    /// Most recently queued buffer id.
    pub last_queued_buffer_id: u32,
    /// Reserved.
    pub unk3: u32,
    /// First-vhandler timestamp low word.
    pub last_vtime_low: u32,
    /// Second-vhandler timestamp.
    pub last_second_vtime: u64,
    /// Reserved.
    pub unk4: u64,
    /// vblank count.
    pub vblank_count: u64,
    /// Reserved.
    pub unk: u32,
    /// First-vhandler timestamp high word.
    pub last_vtime_high: u32,
}

/// Driver info region; 0x12F8 bytes. Fields are BE in guest memory;
/// init writers convert at write time.
#[repr(C)]
pub struct RsxDriverInfo {
    /// Driver version word.
    pub version_driver: u32,
    /// GPU version word.
    pub version_gpu: u32,
    /// RSX-local memory size.
    pub memory_size: u32,
    /// Hardware channel (1 for games, 0 for VSH).
    pub hardware_channel: u32,
    /// nvcore frequency in Hz.
    pub nvcore_frequency: u32,
    /// Memory frequency in Hz.
    pub memory_frequency: u32,
    /// Reserved.
    pub unk1: [u32; 4],
    /// Reserved.
    pub unk2: u32,
    /// Guest-visible offset from reports_base to the notify array.
    pub reports_notify_offset: u32,
    /// Guest-visible offset from reports_base to the semaphore block.
    /// Name is RPCS3's `reportsOffset`; "reports" is overloaded across
    /// three regions.
    pub reports_offset: u32,
    /// Guest-visible offset from reports_base to the report entries.
    pub reports_report_offset: u32,
    /// Reserved.
    pub unk3: [u32; 6],
    /// System-mode flags.
    pub system_mode_flags: u32,
    /// Reserved.
    pub unk4: [u8; 0x1064],
    /// Per-display head states.
    pub head: [RsxDriverHead; 8],
    /// Reserved.
    pub unk7: u32,
    /// Reserved.
    pub unk8: u32,
    /// Handler-presence flags.
    pub handlers: u32,
    /// Reserved.
    pub unk9: u32,
    /// Reserved.
    pub unk10: u32,
    /// User-command callback param.
    pub user_cmd_param: u32,
    /// Event-queue handle at offset 0x12D0; the RSX -> PPU event
    /// delivery substrate guests pass to `sys_event_queue_receive`.
    pub handler_queue: u32,
    /// Reserved.
    pub unk11: u32,
    /// Reserved.
    pub unk12: u32,
    /// Reserved.
    pub unk13: u32,
    /// Reserved.
    pub unk14: u32,
    /// Reserved.
    pub unk15: u32,
    /// Reserved.
    pub unk16: u32,
    /// Reserved.
    pub unk17: [u32; 2],
    /// Last-error word read by `cellGcmSetGraphicsHandler`.
    pub last_error: u32,
}

/// Guest address of label `index`.
///
/// # Panics
///
/// Debug-asserts `index < LABEL_COUNT` and that `reports_base +
/// index * LABEL_STRIDE` does not wrap u32.
#[inline]
pub fn label_address(reports_base: u32, index: u32) -> u32 {
    debug_assert!(
        index < LABEL_COUNT,
        "label index {index} out of range (max {})",
        LABEL_COUNT - 1
    );
    let byte_offset = index * LABEL_STRIDE;
    let addr = reports_base.checked_add(byte_offset);
    debug_assert!(
        addr.is_some(),
        "label address arithmetic wrapped (reports_base {reports_base:#x} + {byte_offset})"
    );
    addr.unwrap_or(0)
}

/// Committed state for the single sys_rsx context.
///
/// Only one context is supported; [`RSX_CONTEXT_ID`] is fixed.
/// Every field folds into [`Self::state_hash`] via destructure, so
/// adding a field is a compile error until the hash is updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxContext {
    /// `sys_rsx_memory_allocate` has fired.
    pub memory_allocated: bool,
    /// `sys_rsx_context_allocate` has fired; requires
    /// `memory_allocated`.
    pub allocated: bool,
    /// Context id returned to the guest (`RSX_CONTEXT_ID` when set).
    pub context_id: u32,
    /// DMA control region base.
    pub dma_control_addr: u32,
    /// Driver info region base.
    pub driver_info_addr: u32,
    /// Reports region base.
    pub reports_addr: u32,
    /// Event-queue handle.
    pub event_queue_id: u32,
    /// Event-port handle.
    pub event_port_id: u32,
    /// Memory-allocate handle.
    pub mem_handle: u32,
    /// Memory-allocate base address.
    pub mem_addr: u32,
}

impl RsxContext {
    /// Pristine state: no allocation, all fields zero.
    #[inline]
    pub const fn new() -> Self {
        Self {
            memory_allocated: false,
            allocated: false,
            context_id: 0,
            dma_control_addr: 0,
            driver_info_addr: 0,
            reports_addr: 0,
            event_queue_id: 0,
            event_port_id: 0,
            mem_handle: 0,
            mem_addr: 0,
        }
    }

    /// Record the result of `sys_rsx_memory_allocate`.
    ///
    /// Must run before [`Self::set_context_allocated`]; the latter
    /// debug-asserts that this one fired.
    ///
    /// # Panics
    ///
    /// Debug-asserts `!self.memory_allocated` and `mem_addr != 0`
    /// (zero is the pristine sentinel).
    pub fn set_memory_allocated(&mut self, mem_handle: u32, mem_addr: u32) {
        debug_assert!(!self.memory_allocated, "set_memory_allocated called twice");
        debug_assert!(
            mem_addr != 0,
            "mem_addr 0 is reserved as the pristine sentinel"
        );
        self.memory_allocated = true;
        self.mem_handle = mem_handle;
        self.mem_addr = mem_addr;
    }

    /// Record the result of `sys_rsx_context_allocate`.
    ///
    /// Derives the three region addresses from `mem_addr + *_OFFSET`.
    ///
    /// # Panics
    ///
    /// Debug-asserts `!self.allocated`, `self.memory_allocated`,
    /// that the full reservation fits in u32 space starting at
    /// `mem_addr`, and that each derived address meets the alignment
    /// its struct requires.
    pub fn set_context_allocated(
        &mut self,
        context_id: u32,
        event_queue_id: u32,
        event_port_id: u32,
    ) {
        debug_assert!(
            !self.allocated,
            "set_context_allocated called twice on the same context"
        );
        debug_assert!(
            self.memory_allocated,
            "set_context_allocated called before set_memory_allocated"
        );
        debug_assert!(
            self.mem_addr
                .checked_add(region::CONTEXT_RESERVATION)
                .is_some(),
            "mem_addr {:#x} + reservation {:#x} wraps u32",
            self.mem_addr,
            region::CONTEXT_RESERVATION
        );
        // dma_control_addr is the fixed MMIO base 0xC0000000 (libgcm
        // adds +0x40 internally to derive the put-pointer write at
        // 0xC0000040). driver_info / reports are RAM-backed inside
        // the per-context reservation.
        let dma_control_addr = control_register::DMA_CONTROL_BASE;
        let driver_info_addr = self.mem_addr + region::DRIVER_INFO_OFFSET;
        let reports_addr = self.mem_addr + region::REPORTS_OFFSET;
        debug_assert!(
            reports_addr.is_multiple_of(16),
            "reports_addr {reports_addr:#x} must be 16-byte aligned for RsxReports"
        );
        debug_assert!(
            driver_info_addr.is_multiple_of(16),
            "driver_info_addr {driver_info_addr:#x} must be 16-byte aligned"
        );
        debug_assert!(
            dma_control_addr.is_multiple_of(4),
            "dma_control_addr {dma_control_addr:#x} must be u32-aligned"
        );
        self.allocated = true;
        self.context_id = context_id;
        self.dma_control_addr = dma_control_addr;
        self.driver_info_addr = driver_info_addr;
        self.reports_addr = reports_addr;
        self.event_queue_id = event_queue_id;
        self.event_port_id = event_port_id;
    }

    /// FNV-1a hash over every field prefixed with
    /// [`STATE_HASH_FORMAT_VERSION`]. Folds into the sync-state hash.
    pub fn state_hash(&self) -> u64 {
        let Self {
            memory_allocated,
            allocated,
            context_id,
            dma_control_addr,
            driver_info_addr,
            reports_addr,
            event_queue_id,
            event_port_id,
            mem_handle,
            mem_addr,
        } = *self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&[STATE_HASH_FORMAT_VERSION]);
        hasher.write(&[u8::from(memory_allocated)]);
        hasher.write(&[u8::from(allocated)]);
        hasher.write(&context_id.to_le_bytes());
        hasher.write(&dma_control_addr.to_le_bytes());
        hasher.write(&driver_info_addr.to_le_bytes());
        hasher.write(&reports_addr.to_le_bytes());
        hasher.write(&event_queue_id.to_le_bytes());
        hasher.write(&event_port_id.to_le_bytes());
        hasher.write(&mem_handle.to_le_bytes());
        hasher.write(&mem_addr.to_le_bytes());
        hasher.finish()
    }
}

impl Default for RsxContext {
    fn default() -> Self {
        Self::new()
    }
}

pub use cellgov_ps3_abi::sys_rsx::{driver_info, reports};

/// Bytes a [`RsxDmaControl`] occupies in guest memory.
pub const RSX_DMA_CONTROL_SIZE: usize = size_of::<RsxDmaControl>();

const _: () = assert!(
    size_of::<RsxReports>() == reports::SIZE,
    "RsxReports layout drift vs cellgov_ps3_abi::sys_rsx::reports::SIZE"
);
const _: () = assert!(
    size_of::<RsxDriverInfo>() == driver_info::SIZE,
    "RsxDriverInfo layout drift vs cellgov_ps3_abi::sys_rsx::driver_info::SIZE"
);

#[cfg(test)]
#[path = "tests/reports_tests.rs"]
mod tests;
