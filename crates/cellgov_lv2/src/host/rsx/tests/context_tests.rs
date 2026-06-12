//! `sys_rsx_context_allocate` dispatch tests: out-pointer writes, reports/driver-info initialization, and recorded context state.

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
        u64::from(control_register::DMA_CONTROL_BASE)
    );
    assert_eq!(
        u64::from(control_register::DMA_CONTROL_BASE) + 0x40,
        u64::from(control_register::PUT_ADDR),
        "dma_control_base + 0x40 must equal PUT_ADDR"
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
    assert_eq!(ctx.dma_control_addr, control_register::DMA_CONTROL_BASE);
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
        Lv2Dispatch::Immediate { code, effects } if code == u64::from(cell_errors::CELL_EINVAL) && effects.is_empty()
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
