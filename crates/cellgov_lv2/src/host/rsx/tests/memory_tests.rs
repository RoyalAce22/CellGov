//! `sys_rsx_memory_allocate` / `sys_rsx_memory_free` dispatch tests: bump allocation from the RSX region base and ENOMEM bounds.

use super::*;
use crate::host::rsx::test_helpers::extract_write_u64;
use crate::host::test_support::{extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;

fn allocate_rsx(host: &mut Lv2Host, size: u32, source: UnitId) -> (u32, u64) {
    let rt = FakeRuntime::new(0x10_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x1000,
            mem_addr_ptr: 0x2000,
            size,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        source,
        &rt,
    );
    match d {
        Lv2Dispatch::Immediate { code: 0, effects } => (
            extract_write_u32(&effects[0]),
            extract_write_u64(&effects[1]),
        ),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn sys_rsx_memory_allocate_returns_base_then_bumps() {
    let mut host = Lv2Host::new();
    let source = UnitId::new(0);

    let (h1, a1) = allocate_rsx(&mut host, 0x30_0000, source);
    assert_eq!(h1, 1);
    assert_eq!(a1, Lv2Host::SYS_RSX_MEM_BASE as u64);

    let (h2, a2) = allocate_rsx(&mut host, 0x30_0000, source);
    assert_eq!(h2, 2);
    assert_eq!(a2, (Lv2Host::SYS_RSX_MEM_BASE + 0x30_0000) as u64);
}

#[test]
fn sys_rsx_memory_allocate_rejects_zero_size() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x1000,
            mem_addr_ptr: 0x2000,
            size: 0,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_ENOMEM)
    ));
}

#[test]
fn sys_rsx_memory_allocate_rejects_beyond_region_end() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x1000,
            mem_addr_ptr: 0x2000,
            size: 0x2000_0000,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_ENOMEM)
    ));
}

#[test]
fn sys_rsx_memory_free_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let d = host.dispatch(
        Lv2Request::SysRsxMemoryFree { mem_handle: 0xA001 },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()
    ));
}
