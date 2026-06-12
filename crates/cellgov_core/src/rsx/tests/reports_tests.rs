//! Guest-facing RSX report and driver-info layout pins plus RsxContext allocation-state hashing.

use super::*;
use core::mem::offset_of;

#[test]
fn rsx_reports_size_matches_rpcs3() {
    assert_eq!(reports::SIZE, 0x9400);
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
    assert_eq!(driver_info::SIZE, 0x12F8);
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
    assert_eq!(region::DRIVER_INFO_OFFSET, 0x0010_0000);
    assert_eq!(region::REPORTS_OFFSET, 0x0020_0000);
    assert_eq!(region::CONTEXT_RESERVATION, 0x0030_0000);
    // u64 guards against future sizes truncating via `as u32`.
    let dri = driver_info::SIZE as u64;
    let rep = reports::SIZE as u64;
    assert!(u64::from(region::REPORTS_OFFSET) >= u64::from(region::DRIVER_INFO_OFFSET) + dri);
    assert!(u64::from(region::REPORTS_OFFSET) + rep <= u64::from(region::CONTEXT_RESERVATION));
}

#[test]
fn dma_control_base_plus_offset_equals_put_addr() {
    assert_eq!(
        u64::from(control_register::DMA_CONTROL_BASE) + 0x40,
        u64::from(control_register::PUT_ADDR),
        "dma_control_base + 0x40 must equal PUT_ADDR"
    );
    assert_eq!(control_register::DMA_CONTROL_BASE, 0xC000_0000);
    assert_eq!(control_register::PUT_ADDR, 0xC000_0040);
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
    assert_eq!(ctx.dma_control_addr, control_register::DMA_CONTROL_BASE);
    assert_eq!(
        ctx.driver_info_addr,
        0x3000_0000 + region::DRIVER_INFO_OFFSET
    );
    assert_eq!(ctx.reports_addr, 0x3000_0000 + region::REPORTS_OFFSET);
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
