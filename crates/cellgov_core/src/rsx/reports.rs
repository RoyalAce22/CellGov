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

pub use cellgov_lv2::host::rsx::{
    driver_info_init, DMA_CONTROL_OFFSET, DRIVER_INFO_OFFSET, REPORTS_OFFSET, RSX_CONTEXT_ID,
    RSX_CONTEXT_RESERVATION, SEMAPHORE_INIT_PATTERN,
};

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

/// Bytes between consecutive labels in the semaphore region.
pub const LABEL_STRIDE: u32 = 0x10;

/// Count of addressable labels.
pub const LABEL_COUNT: u32 = 256;

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
            self.mem_addr.checked_add(RSX_CONTEXT_RESERVATION).is_some(),
            "mem_addr {:#x} + reservation {:#x} wraps u32",
            self.mem_addr,
            RSX_CONTEXT_RESERVATION
        );
        let dma_control_addr = self.mem_addr + DMA_CONTROL_OFFSET;
        let driver_info_addr = self.mem_addr + DRIVER_INFO_OFFSET;
        let reports_addr = self.mem_addr + REPORTS_OFFSET;
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

pub use cellgov_lv2::host::rsx::{RSX_DRIVER_INFO_SIZE, RSX_REPORTS_SIZE};

/// Bytes a [`RsxDmaControl`] occupies in guest memory.
pub const RSX_DMA_CONTROL_SIZE: usize = size_of::<RsxDmaControl>();

const _: () = assert!(
    size_of::<RsxReports>() == RSX_REPORTS_SIZE,
    "RsxReports layout drift vs cellgov_lv2::host::rsx::RSX_REPORTS_SIZE"
);
const _: () = assert!(
    size_of::<RsxDriverInfo>() == RSX_DRIVER_INFO_SIZE,
    "RsxDriverInfo layout drift vs cellgov_lv2::host::rsx::RSX_DRIVER_INFO_SIZE"
);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn rsx_reports_size_matches_rpcs3() {
        assert_eq!(RSX_REPORTS_SIZE, 0x9400);
    }

    #[test]
    fn rsx_reports_notify_offset_is_1000() {
        assert_eq!(offset_of!(RsxReports, notify), 0x1000);
    }

    #[test]
    fn rsx_reports_report_offset_is_1400() {
        assert_eq!(offset_of!(RsxReports, report), 0x1400);
    }

    #[test]
    fn rsx_notify_size_and_alignment() {
        assert_eq!(size_of::<RsxNotify>(), 16);
        assert_eq!(core::mem::align_of::<RsxNotify>(), 16);
    }

    #[test]
    fn rsx_report_size_and_alignment() {
        assert_eq!(size_of::<RsxReport>(), 16);
        assert_eq!(core::mem::align_of::<RsxReport>(), 16);
    }

    #[test]
    fn rsx_dma_control_total_size() {
        assert_eq!(RSX_DMA_CONTROL_SIZE, 0x58);
    }

    #[test]
    fn rsx_dma_control_put_get_ref_offsets() {
        assert_eq!(offset_of!(RsxDmaControl, put), 0x40);
        assert_eq!(offset_of!(RsxDmaControl, get), 0x44);
        assert_eq!(offset_of!(RsxDmaControl, ref_value), 0x48);
    }

    #[test]
    fn rsx_dma_control_reserved_tail_offsets() {
        assert_eq!(offset_of!(RsxDmaControl, unk), 0x4C);
        assert_eq!(offset_of!(RsxDmaControl, unk1), 0x54);
    }

    #[test]
    fn rsx_driver_info_size_matches_rpcs3() {
        assert_eq!(RSX_DRIVER_INFO_SIZE, 0x12F8);
    }

    #[test]
    fn rsx_driver_info_head_is_at_10b8() {
        assert_eq!(offset_of!(RsxDriverInfo, head), 0x10B8);
    }

    #[test]
    fn rsx_driver_info_handler_queue_is_at_12d0() {
        assert_eq!(offset_of!(RsxDriverInfo, handler_queue), 0x12D0);
    }

    #[test]
    fn rsx_driver_info_guest_facing_field_offsets() {
        assert_eq!(offset_of!(RsxDriverInfo, version_driver), 0x00);
        assert_eq!(offset_of!(RsxDriverInfo, version_gpu), 0x04);
        assert_eq!(offset_of!(RsxDriverInfo, memory_size), 0x08);
        assert_eq!(offset_of!(RsxDriverInfo, hardware_channel), 0x0C);
        assert_eq!(offset_of!(RsxDriverInfo, nvcore_frequency), 0x10);
        assert_eq!(offset_of!(RsxDriverInfo, memory_frequency), 0x14);
        assert_eq!(offset_of!(RsxDriverInfo, reports_notify_offset), 0x2C);
        assert_eq!(offset_of!(RsxDriverInfo, reports_offset), 0x30);
        assert_eq!(offset_of!(RsxDriverInfo, reports_report_offset), 0x34);
        assert_eq!(offset_of!(RsxDriverInfo, system_mode_flags), 0x50);
        assert_eq!(offset_of!(RsxDriverInfo, handlers), 0x12C0);
        assert_eq!(offset_of!(RsxDriverInfo, user_cmd_param), 0x12CC);
        assert_eq!(offset_of!(RsxDriverInfo, last_error), 0x12F4);
    }

    #[test]
    fn reports_notify_offset_matches_driver_info_constant() {
        assert_eq!(
            offset_of!(RsxReports, notify) as u32,
            driver_info_init::REPORTS_NOTIFY_OFFSET
        );
        assert_eq!(
            offset_of!(RsxReports, report) as u32,
            driver_info_init::REPORTS_REPORT_OFFSET
        );
    }

    #[test]
    fn rsx_driver_info_head_size_is_40() {
        assert_eq!(size_of::<RsxDriverHead>(), 0x40);
    }

    #[test]
    fn rsx_context_new_is_pristine() {
        let ctx = RsxContext::new();
        assert!(!ctx.memory_allocated);
        assert!(!ctx.allocated);
        assert_eq!(ctx.context_id, 0);
        assert_eq!(ctx.dma_control_addr, 0);
        assert_eq!(ctx.driver_info_addr, 0);
        assert_eq!(ctx.reports_addr, 0);
        assert_eq!(ctx.event_queue_id, 0);
        assert_eq!(ctx.event_port_id, 0);
        assert_eq!(ctx.mem_handle, 0);
        assert_eq!(ctx.mem_addr, 0);
    }

    #[test]
    fn reservation_offsets_match_rpcs3_layout() {
        assert_eq!(DMA_CONTROL_OFFSET, 0x0000_0000);
        assert_eq!(DRIVER_INFO_OFFSET, 0x0010_0000);
        assert_eq!(REPORTS_OFFSET, 0x0020_0000);
        assert_eq!(RSX_CONTEXT_RESERVATION, 0x0030_0000);
        // u64 guards against future sizes truncating via `as u32`.
        let dma = RSX_DMA_CONTROL_SIZE as u64;
        let dri = RSX_DRIVER_INFO_SIZE as u64;
        let rep = RSX_REPORTS_SIZE as u64;
        assert!(u64::from(DRIVER_INFO_OFFSET) >= u64::from(DMA_CONTROL_OFFSET) + dma);
        assert!(u64::from(REPORTS_OFFSET) >= u64::from(DRIVER_INFO_OFFSET) + dri);
        assert!(u64::from(REPORTS_OFFSET) + rep <= u64::from(RSX_CONTEXT_RESERVATION));
    }

    #[test]
    fn rsx_context_pristine_state_hash_golden() {
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(&[STATE_HASH_FORMAT_VERSION]);
        h.write(&[u8::from(false)]);
        h.write(&[u8::from(false)]);
        for _ in 0..8 {
            h.write(&0u32.to_le_bytes());
        }
        assert_eq!(RsxContext::new().state_hash(), h.finish());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "pristine sentinel")]
    fn set_memory_allocated_rejects_zero_mem_addr() {
        let mut ctx = RsxContext::new();
        ctx.set_memory_allocated(0xA001, 0);
    }

    #[test]
    fn rsx_context_state_hash_deterministic() {
        let a = RsxContext::new();
        let b = RsxContext::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn rsx_context_state_hash_distinguishes_every_field() {
        fn hash_with(f: impl FnOnce(&mut RsxContext)) -> u64 {
            let mut ctx = RsxContext::new();
            f(&mut ctx);
            ctx.state_hash()
        }
        let base = hash_with(|_| {});
        assert_ne!(base, hash_with(|c| c.memory_allocated = true));
        assert_ne!(base, hash_with(|c| c.allocated = true));
        assert_ne!(base, hash_with(|c| c.context_id = 1));
        assert_ne!(base, hash_with(|c| c.dma_control_addr = 1));
        assert_ne!(base, hash_with(|c| c.driver_info_addr = 1));
        assert_ne!(base, hash_with(|c| c.reports_addr = 1));
        assert_ne!(base, hash_with(|c| c.event_queue_id = 1));
        assert_ne!(base, hash_with(|c| c.event_port_id = 1));
        assert_ne!(base, hash_with(|c| c.mem_handle = 1));
        assert_ne!(base, hash_with(|c| c.mem_addr = 1));
    }

    #[test]
    fn semaphore_init_pattern_matches_rpcs3() {
        assert_eq!(SEMAPHORE_INIT_PATTERN[0], 0x1337_C0D3);
        assert_eq!(SEMAPHORE_INIT_PATTERN[1], 0x1337_BABE);
        assert_eq!(SEMAPHORE_INIT_PATTERN[2], 0x1337_BEEF);
        assert_eq!(SEMAPHORE_INIT_PATTERN[3], 0x1337_F001);
    }

    #[test]
    fn label_stride_maps_label_index_to_sentinel_correctly() {
        assert_eq!(LABEL_STRIDE, 0x10);
        assert_eq!(LABEL_COUNT, 256);
        assert_eq!(LABEL_COUNT * LABEL_STRIDE, 4096);

        for i in 0..LABEL_COUNT {
            let byte_offset = i * LABEL_STRIDE;
            let sem_index = (byte_offset / 4) as usize;
            assert!(sem_index < 1024);
            let expected = SEMAPHORE_INIT_PATTERN[sem_index % 4];
            if i == 255 {
                assert_eq!(sem_index, 1020);
                assert_eq!(expected, 0x1337_C0D3);
            }
        }
    }

    #[test]
    fn rsx_context_memory_then_context_allocation_records_all_fields() {
        let mut ctx = RsxContext::new();
        ctx.set_memory_allocated(0xA001, 0x3000_0000);
        assert!(ctx.memory_allocated);
        assert!(!ctx.allocated);
        assert_eq!(ctx.mem_handle, 0xA001);
        assert_eq!(ctx.mem_addr, 0x3000_0000);

        ctx.set_context_allocated(RSX_CONTEXT_ID, 0xE001, 0xE002);

        assert!(ctx.allocated);
        assert_eq!(ctx.context_id, RSX_CONTEXT_ID);
        assert_eq!(ctx.dma_control_addr, 0x3000_0000 + DMA_CONTROL_OFFSET);
        assert_eq!(ctx.driver_info_addr, 0x3000_0000 + DRIVER_INFO_OFFSET);
        assert_eq!(ctx.reports_addr, 0x3000_0000 + REPORTS_OFFSET);
        assert_eq!(ctx.event_queue_id, 0xE001);
        assert_eq!(ctx.event_port_id, 0xE002);
        assert_eq!(ctx.mem_handle, 0xA001);
        assert_eq!(ctx.mem_addr, 0x3000_0000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "set_context_allocated called before set_memory_allocated")]
    fn rsx_context_set_context_before_memory_panics() {
        let mut ctx = RsxContext::new();
        ctx.set_context_allocated(RSX_CONTEXT_ID, 0, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "twice on the same context")]
    fn rsx_context_double_context_allocate_panics() {
        let mut ctx = RsxContext::new();
        ctx.set_memory_allocated(0xA001, 0x3000_0000);
        ctx.set_context_allocated(RSX_CONTEXT_ID, 0, 0);
        ctx.set_context_allocated(RSX_CONTEXT_ID, 0, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "set_memory_allocated called twice")]
    fn rsx_context_double_memory_allocate_panics() {
        let mut ctx = RsxContext::new();
        ctx.set_memory_allocated(0xA001, 0x3000_0000);
        ctx.set_memory_allocated(0xA002, 0x4000_0000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "aligned")]
    fn rsx_context_set_context_rejects_unaligned_derived_address() {
        let mut ctx = RsxContext::new();
        ctx.set_memory_allocated(0xA001, 0x0000_0004);
        ctx.set_context_allocated(RSX_CONTEXT_ID, 0, 0);
    }

    #[test]
    fn label_address_helper_matches_manual_arithmetic() {
        let base = 0x3020_0000u32;
        assert_eq!(label_address(base, 0), base);
        assert_eq!(label_address(base, 1), base + 0x10);
        assert_eq!(label_address(base, 255), base + 0xFF0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out of range")]
    fn label_address_helper_rejects_index_256() {
        let _ = label_address(0x3020_0000, 256);
    }

    #[test]
    fn rsx_context_fully_populated_state_hash_golden() {
        let mut ctx = RsxContext::new();
        ctx.memory_allocated = true;
        ctx.allocated = true;
        ctx.context_id = 0x1111_1111;
        ctx.dma_control_addr = 0x2222_2222;
        ctx.driver_info_addr = 0x3333_3333;
        ctx.reports_addr = 0x4444_4440;
        ctx.event_queue_id = 0x5555_5555;
        ctx.event_port_id = 0x6666_6666;
        ctx.mem_handle = 0x7777_7777;
        ctx.mem_addr = 0x8888_8880;

        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(&[STATE_HASH_FORMAT_VERSION]);
        h.write(&[u8::from(true)]); // memory_allocated
        h.write(&[u8::from(true)]); // allocated
        h.write(&0x1111_1111u32.to_le_bytes());
        h.write(&0x2222_2222u32.to_le_bytes());
        h.write(&0x3333_3333u32.to_le_bytes());
        h.write(&0x4444_4440u32.to_le_bytes());
        h.write(&0x5555_5555u32.to_le_bytes());
        h.write(&0x6666_6666u32.to_le_bytes());
        h.write(&0x7777_7777u32.to_le_bytes());
        h.write(&0x8888_8880u32.to_le_bytes());

        assert_eq!(ctx.state_hash(), h.finish());
    }
}
