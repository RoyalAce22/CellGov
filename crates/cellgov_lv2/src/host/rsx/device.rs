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
#[path = "tests/device_tests.rs"]
mod tests;
