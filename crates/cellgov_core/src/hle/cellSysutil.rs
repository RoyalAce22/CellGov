//! cellSysutil HLE implementations.
//!
//! Covers the video-out query surface a title's display-init code
//! consults during boot. CellGov reports a primary 720p RGB display
//! deterministically; titles that read these structs and treat zero
//! as "no display attached" advance into normal init instead.
//!
//! ## Failure policy
//!
//! - Null guest out-pointers return `error::ILLEGAL_PARAMETER`
//!   without writing anything (matches RPCS3's `vm::ptr` null guard).
//! - Out-of-range `videoOut` returns `error::UNSUPPORTED_VIDEO_OUT`.
//! - `deviceIndex` out of range for the chosen `videoOut` returns
//!   `error::DEVICE_NOT_FOUND` (CellGov models exactly
//!   one device on `display::PRIMARY`, zero on
//!   `display::SECONDARY`).

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_video_out::{
    aspect, color_space, display, display_conversion, display_mode_resolution, error, output_state,
    refresh_rate, resolution_id, scan_mode,
};
use cellgov_ps3_abi::nid::cell_sysutil as sysutil_nid;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

/// Every NID this module claims; sourced from
/// [`cellgov_ps3_abi::nid::cell_sysutil::OWNED`].
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = sysutil_nid::OWNED;

/// Dispatch entry point; returns `None` if the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        sysutil_nid::VIDEO_OUT_GET_STATE => {
            video_out_get_state(&mut adapter(runtime, source, nid), args);
        }
        sysutil_nid::VIDEO_OUT_GET_RESOLUTION => {
            video_out_get_resolution(&mut adapter(runtime, source, nid), args);
        }
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId, nid: u32) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_base: runtime.hle.heap_base,
        heap_ptr: &mut runtime.hle.heap_ptr,
        heap_watermark: &mut runtime.hle.heap_watermark,
        heap_warning_mask: &mut runtime.hle.heap_warning_mask,
        next_id: &mut runtime.hle.next_id,
        source,
        nid,
        mutated: false,
        handlers_without_mutation: &mut runtime.hle.handlers_without_mutation,
    }
}

/// `cellVideoOutGetState(videoOut, deviceIndex, state)`.
///
/// Reports a primary 720p / RGB / 16:9 / 59.94Hz display when the
/// caller queries `display::PRIMARY` with `deviceIndex == 0`.
/// `display::SECONDARY` is modelled as having no devices
/// attached. Any other `videoOut` value is unsupported.
pub(crate) fn video_out_get_state(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let video_out = args[1] as u32;
    let device_index = args[2] as u32;
    let state_ptr = args[3] as u32;

    if state_ptr == 0 {
        ctx.set_return(error::ILLEGAL_PARAMETER as u64);
        return;
    }

    if video_out != display::PRIMARY && video_out != display::SECONDARY {
        ctx.set_return(error::UNSUPPORTED_VIDEO_OUT as u64);
        return;
    }

    // PRIMARY has exactly one device (index 0); SECONDARY has none.
    let device_count = if video_out == display::PRIMARY { 1 } else { 0 };
    if device_index >= device_count {
        ctx.set_return(error::DEVICE_NOT_FOUND as u64);
        return;
    }

    // Faithful PRIMARY / 720p / RGB / 16:9 / 59.94Hz baseline.
    // Layout follows `CellVideoOutState` in cellVideoOut.h:
    //   u8 state, u8 colorSpace, u8 reserved[6],
    //   CellVideoOutDisplayMode displayMode (8 bytes).
    // CellVideoOutDisplayMode:
    //   u8 resolutionId, u8 scanMode, u8 conversion, u8 aspect,
    //   u8 reserved[2], be_t<u16> refreshRates.
    let mut buf = [0u8; 16];
    buf[0] = output_state::ENABLED;
    buf[1] = color_space::RGB;
    // buf[2..8] reserved zeros.
    buf[8] = display_mode_resolution::ID_720;
    buf[9] = scan_mode::PROGRESSIVE;
    buf[10] = display_conversion::NONE;
    buf[11] = aspect::WIDE_16_9;
    // buf[12..14] reserved zeros.
    buf[14..16].copy_from_slice(&refresh_rate::HZ_59_94.to_be_bytes());

    ctx.write_guest(state_ptr as u64, &buf)
        .expect("cellVideoOutGetState: write to caller out-ptr failed");
    ctx.set_return(0);
}

/// `cellVideoOutGetResolution(resolutionId, resolution)`.
///
/// Looks the resolution ID up in the PS3 spec table and writes the
/// (width, height) pair as two big-endian `u16`s. Unknown IDs return
/// `error::ILLEGAL_PARAMETER` without writing the
/// out-pointer.
pub(crate) fn video_out_get_resolution(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let resolution_id = args[1] as u32;
    let resolution_ptr = args[2] as u32;

    if resolution_ptr == 0 {
        ctx.set_return(error::ILLEGAL_PARAMETER as u64);
        return;
    }

    let Some((width, height)) = resolution_lookup(resolution_id) else {
        ctx.set_return(error::ILLEGAL_PARAMETER as u64);
        return;
    };

    let mut buf = [0u8; 4];
    buf[0..2].copy_from_slice(&width.to_be_bytes());
    buf[2..4].copy_from_slice(&height.to_be_bytes());
    ctx.write_guest(resolution_ptr as u64, &buf)
        .expect("cellVideoOutGetResolution: write to caller out-ptr failed");
    ctx.set_return(0);
}

/// PS3 spec resolution table.
///
/// Values mirror `_IntGetResolutionInfo` in
/// `tools/rpcs3-src/rpcs3/Emu/Cell/Modules/cellVideoOut.cpp`. Only
/// the standard 2D resolutions are supported; 3D / dualview / unusual
/// IDs return `None` and become `error::ILLEGAL_PARAMETER`
/// at the caller (deferred until a foundation title surfaces one).
fn resolution_lookup(id: u32) -> Option<(u16, u16)> {
    match id {
        resolution_id::ID_1080 => Some((1920, 1080)),
        resolution_id::ID_720 => Some((1280, 720)),
        resolution_id::ID_480 => Some((720, 480)),
        resolution_id::ID_576 => Some((720, 576)),
        resolution_id::ID_1600X1080 => Some((1600, 1080)),
        resolution_id::ID_1440X1080 => Some((1440, 1080)),
        resolution_id::ID_1280X1080 => Some((1280, 1080)),
        resolution_id::ID_960X1080 => Some((960, 1080)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn fixture() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x20_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    fn read_guest_u8(rt: &Runtime, addr: u32) -> u8 {
        rt.memory().as_bytes()[addr as usize]
    }

    fn read_guest_u16_be(rt: &Runtime, addr: u32) -> u16 {
        let m = rt.memory().as_bytes();
        let a = addr as usize;
        u16::from_be_bytes([m[a], m[a + 1]])
    }

    fn read_syscall_return(rt: &mut Runtime, unit_id: UnitId) -> u64 {
        rt.registry_mut()
            .drain_syscall_return(unit_id)
            .expect("handler must set syscall return")
    }

    #[test]
    fn video_out_get_state_primary_writes_720p_rgb_state() {
        let (mut rt, unit_id) = fixture();
        let state_ptr: u32 = 0x10_1000;
        let args: [u64; 9] = [
            0x10000,
            display::PRIMARY as u64,
            0,
            state_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_STATE, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            0,
            "CELL_OK on success"
        );
        assert_eq!(read_guest_u8(&rt, state_ptr), output_state::ENABLED);
        assert_eq!(read_guest_u8(&rt, state_ptr + 1), color_space::RGB);
        for off in 2..8 {
            assert_eq!(read_guest_u8(&rt, state_ptr + off), 0, "reserved[{off}]");
        }
        assert_eq!(
            read_guest_u8(&rt, state_ptr + 8),
            display_mode_resolution::ID_720
        );
        assert_eq!(read_guest_u8(&rt, state_ptr + 9), scan_mode::PROGRESSIVE);
        assert_eq!(read_guest_u8(&rt, state_ptr + 10), display_conversion::NONE);
        assert_eq!(read_guest_u8(&rt, state_ptr + 11), aspect::WIDE_16_9);
        assert_eq!(read_guest_u8(&rt, state_ptr + 12), 0);
        assert_eq!(read_guest_u8(&rt, state_ptr + 13), 0);
        assert_eq!(
            read_guest_u16_be(&rt, state_ptr + 14),
            refresh_rate::HZ_59_94
        );
    }

    #[test]
    fn video_out_get_state_null_state_pointer_returns_illegal_parameter() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, display::PRIMARY as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_STATE, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::ILLEGAL_PARAMETER as u64
        );
    }

    #[test]
    fn video_out_get_state_unsupported_video_out_rejected() {
        let (mut rt, unit_id) = fixture();
        let state_ptr: u32 = 0x10_1000;
        let args: [u64; 9] = [0x10000, 2, 0, state_ptr as u64, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_STATE, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::UNSUPPORTED_VIDEO_OUT as u64,
            "videoOut=2 is neither PRIMARY nor SECONDARY"
        );
        // The error path must not write to the out-pointer.
        assert_eq!(read_guest_u8(&rt, state_ptr), 0);
    }

    #[test]
    fn video_out_get_state_secondary_no_device_returns_device_not_found() {
        let (mut rt, unit_id) = fixture();
        let state_ptr: u32 = 0x10_1000;
        let args: [u64; 9] = [
            0x10000,
            display::SECONDARY as u64,
            0,
            state_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_STATE, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::DEVICE_NOT_FOUND as u64,
            "SECONDARY has no devices attached"
        );
    }

    #[test]
    fn video_out_get_resolution_table_round_trip() {
        let cases: &[(u32, u16, u16)] = &[
            (resolution_id::ID_1080, 1920, 1080),
            (resolution_id::ID_720, 1280, 720),
            (resolution_id::ID_480, 720, 480),
            (resolution_id::ID_576, 720, 576),
            (resolution_id::ID_1600X1080, 1600, 1080),
            (resolution_id::ID_1440X1080, 1440, 1080),
            (resolution_id::ID_1280X1080, 1280, 1080),
            (resolution_id::ID_960X1080, 960, 1080),
        ];
        for &(id, expected_w, expected_h) in cases {
            let (mut rt, unit_id) = fixture();
            let res_ptr: u32 = 0x10_2000;
            let args: [u64; 9] = [0x10000, id as u64, res_ptr as u64, 0, 0, 0, 0, 0, 0];
            rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_RESOLUTION, &args);

            assert_eq!(
                read_syscall_return(&mut rt, unit_id),
                0,
                "id 0x{id:x}: CELL_OK"
            );
            let w = read_guest_u16_be(&rt, res_ptr);
            let h = read_guest_u16_be(&rt, res_ptr + 2);
            assert_eq!((w, h), (expected_w, expected_h), "id 0x{id:x}");
        }
    }

    #[test]
    fn video_out_get_resolution_unknown_id_rejected() {
        let (mut rt, unit_id) = fixture();
        let res_ptr: u32 = 0x10_2000;
        // Resolution id 0xff is not in the spec table.
        let args: [u64; 9] = [0x10000, 0xff, res_ptr as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_RESOLUTION, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::ILLEGAL_PARAMETER as u64
        );
        assert_eq!(read_guest_u16_be(&rt, res_ptr), 0, "no write on error");
        assert_eq!(read_guest_u16_be(&rt, res_ptr + 2), 0, "no write on error");
    }

    #[test]
    fn video_out_get_resolution_null_pointer_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, resolution_id::ID_720 as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_RESOLUTION, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::ILLEGAL_PARAMETER as u64
        );
    }

    #[test]
    fn video_out_get_state_primary_device_index_out_of_range_rejected() {
        let (mut rt, unit_id) = fixture();
        let state_ptr: u32 = 0x10_1000;
        let args: [u64; 9] = [
            0x10000,
            display::PRIMARY as u64,
            1,
            state_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, sysutil_nid::VIDEO_OUT_GET_STATE, &args);

        assert_eq!(
            read_syscall_return(&mut rt, unit_id),
            error::DEVICE_NOT_FOUND as u64,
            "PRIMARY has exactly one device (index 0)"
        );
    }
}

#[cfg(test)]
mod canary_tests {
    use super::{dispatch, OWNED_NIDS};
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn canary_runtime() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x20_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    /// Drift canary for [`OWNED_NIDS`] vs the [`dispatch`] match arms.
    /// Mirror of the same-named tests in cell_gcm_sys / sys_prx_for_user.
    #[test]
    fn owned_nids_all_claimed_by_dispatch() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            // Provide a valid out-pointer so handlers do not bail
            // before set_return / write_guest fire (RuntimeHleAdapter
            // Drop guard requires at least one mutation).
            let state_ptr: u32 = 0x10_1000;
            let args: [u64; 9] = [0x10000, 0, 0, state_ptr as u64, 0, 0, 0, 0, 0];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result,
                Some(()),
                "cell_sysutil::dispatch returned None for NID {nid:#010x} listed in OWNED_NIDS"
            );
        }
    }

    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            cellgov_ps3_abi::nid::sys_prx_for_user::MALLOC,
            cellgov_ps3_abi::nid::cell_gcm_sys::INIT_BODY,
            0xDEAD_BEEF,
        ];
        for &nid in probes {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "cell_sysutil::dispatch claimed NID {nid:#010x} not in OWNED_NIDS"
            );
        }
    }
}
