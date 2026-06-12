//! `sys_rsx_device_map` dispatch tests: BE byte layout of the device-address write, dev-id validation, and idempotence.

use super::*;
use crate::host::rsx::test_helpers::extract_write_u64;
use crate::host::test_support::FakeRuntime;
use crate::request::Lv2Request;

fn dispatch_device_map(
    host: &mut Lv2Host,
    dev_addr_ptr: u32,
    a2_ptr: u32,
    dev_id: u32,
) -> Lv2Dispatch {
    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxDeviceMap {
            dev_addr_ptr,
            a2_ptr,
            dev_id,
        },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn sys_rsx_device_map_dev_id_8_writes_rsx_device_addr_only_and_returns_ok() {
    let mut host = Lv2Host::new();
    let d = dispatch_device_map(&mut host, 0x1000, 0x1008, 8);
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, u64::from(cell_errors::CELL_OK));
    assert_eq!(effects.len(), 1);
    assert_eq!(extract_write_u64(&effects[0]), u64::from(device_map::ADDR));
}

#[test]
fn sys_rsx_device_map_never_writes_a2_regardless_of_pointer() {
    let mut host = Lv2Host::new();
    // 0xd003ed48 is the real-libgcm value observed in
    // RPCS3 issue #2401; pinning it guards against a future
    // change adding an a2 write because the title's pointer
    // happens to look "valid."
    for a2_ptr in [0, 0x1008, 0xd003ed48_u64 as u32] {
        let d = dispatch_device_map(&mut host, 0x1000, a2_ptr, 8);
        let Lv2Dispatch::Immediate { effects, .. } = d else {
            panic!("expected Immediate, got {d:?}");
        };
        assert_eq!(effects.len(), 1, "a2_ptr={a2_ptr:#x}");
    }
}

#[test]
fn sys_rsx_device_map_emits_be_byte_layout_for_low_32_lwz_read() {
    // Cross-module contract with libgcm: vaddr 0x6b4 reads
    // bytes +4..+8 of the OUT slot via Lwz. The 8-byte BE u64
    // store must place the address in the low 32 bits.
    let mut host = Lv2Host::new();
    let d = dispatch_device_map(&mut host, 0x2000, 0x2008, 8);
    let Lv2Dispatch::Immediate { effects, .. } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
        panic!("expected SharedWriteIntent for dev_addr");
    };
    let b = bytes.bytes();
    assert_eq!(b.len(), 8);
    assert_eq!(&b[0..4], &[0, 0, 0, 0]);
    let low_32 = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    assert_eq!(low_32, device_map::ADDR);
}

#[test]
fn sys_rsx_device_map_dev_id_8_is_idempotent_across_calls() {
    let mut host = Lv2Host::new();
    for _ in 0..4 {
        let d = dispatch_device_map(&mut host, 0x1000, 0x1008, 8);
        let Lv2Dispatch::Immediate { code, effects } = d else {
            panic!("expected Immediate, got {d:?}");
        };
        assert_eq!(code, u64::from(cell_errors::CELL_OK));
        assert_eq!(effects.len(), 1);
        assert_eq!(extract_write_u64(&effects[0]), u64::from(device_map::ADDR));
    }
}

#[test]
fn sys_rsx_device_map_dev_id_not_8_returns_einval_and_bumps_count() {
    let mut host = Lv2Host::new();
    let breaks_before = host.invariant_break_count();
    for bad_dev_id in [0, 1, 7, 9, 11, u32::MAX] {
        let d = dispatch_device_map(&mut host, 0x1000, 0x1008, bad_dev_id);
        let Lv2Dispatch::Immediate { code, effects } = &d else {
            panic!("dev_id {bad_dev_id}: expected Immediate, got {d:?}");
        };
        assert_eq!(
            *code,
            u64::from(cell_errors::CELL_EINVAL),
            "dev_id {bad_dev_id}"
        );
        assert!(effects.is_empty(), "dev_id {bad_dev_id}");
    }
    assert_eq!(host.invariant_break_count() - breaks_before, 6);
}

#[test]
fn rsx_device_addr_value_is_within_rpcs3_documented_range() {
    // RPCS3's sys_rsx.cpp documents dev_addr in
    // 0x40000000..0xB0000000; this anchor catches a future
    // change that moves it out of the range libgcm expects.
    assert_ne!(device_map::ADDR, 0);
    assert!((0x4000_0000..0xB000_0000).contains(&device_map::ADDR));
}

#[test]
fn sys_rsx_device_map_null_dev_addr_returns_efault_and_emits_no_writes() {
    let mut host = Lv2Host::new();
    let breaks_before = host.invariant_break_count();
    let d = dispatch_device_map(&mut host, 0, 0x1008, 8);
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(
        code,
        u64::from(cell_errors::CELL_EFAULT),
        "null dev_addr_ptr must yield CELL_EFAULT, not CELL_OK"
    );
    assert!(
        effects.is_empty(),
        "no SharedWriteIntent may be emitted on the null-pointer EFAULT path; \
         got effects: {effects:?}"
    );
    assert_eq!(
        host.invariant_break_count() - breaks_before,
        1,
        "the null-pointer EFAULT path must log_invariant_break exactly once \
         so release builds surface the case the prior debug_assert hid"
    );
}
