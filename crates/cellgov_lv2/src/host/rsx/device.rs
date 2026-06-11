//! `sys_rsx_device_map` (675) dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_rsx::device_map;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

const _: () = assert!(
    device_map::ADDR + device_map::RESERVATION_SIZE <= Lv2Host::MMAPPER_REGION_START,
    "device_map::ADDR + RESERVATION_SIZE must end at or below \
     MMAPPER_REGION_START so device-map and mmapper allocations never alias"
);

impl Lv2Host {
    /// `sys_rsx_device_map` (675): write [`device_map::ADDR`] as 8-byte BE u64
    /// into `dev_addr` OUT for `dev_id == 8`. `a2` is documented unused and
    /// left untouched. Idempotent across repeated calls.
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` for any `dev_id` other than `8`.
    pub(in crate::host) fn dispatch_sys_rsx_device_map(
        &mut self,
        dev_addr_ptr: u32,
        _a2_ptr: u32,
        dev_id: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if dev_id != 8 {
            self.log_invariant_break(
                "dispatch.sys_rsx_device_map_unsupported_dev_id",
                format_args!(
                    "sys_rsx_device_map dev_id={dev_id} not modeled \
                     (cellGcmInitPerfMon uses 7/9/10/11/12); returning CELL_EINVAL"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if dev_addr_ptr == 0 {
            self.log_invariant_break(
                "dispatch.sys_rsx_device_map_null_dev_addr",
                format_args!(
                    "sys_rsx_device_map dev_addr OUT pointer is null; the 8-byte \
                     device address write at addr 0 would silently clobber the readable \
                     main region. Returning CELL_EFAULT (vm::ptr<u64> rejects null in RPCS3)."
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let device_addr = u64::from(device_map::ADDR);
        let dev_addr_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(dev_addr_ptr, 8),
            bytes: WritePayload::from_slice(&device_addr.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: cell_errors::CELL_OK.into(),
            effects: vec![dev_addr_write],
        }
    }
}

#[cfg(test)]
mod tests {
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
}
