//! Hypercall 210, `gpu_open`: lv1 opens the GPU for the calling
//! partition.
//!
//! The handler answers the call at 4.93; see [`cellgov_ps3_abi::lv1::gpu`].

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::call::GPU_OPEN;
use cellgov_ps3_abi::lv1::gpu::{
    gpu_open_handler, DEFAULT_BOOT_MODE, DEFAULT_RSX_ENABLE, GPU_OPEN_BOOT_MODE_MAX,
    GPU_OPEN_BOOT_MODE_R3, GPU_OPEN_BUSY_R3, GPU_OPEN_DISABLED_R3, GPU_OPEN_RSX_ENABLE_MIN,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::{GpuChange, GpuState};
use crate::answer::{HypercallAnswer, HypercallDisposition, UnmodelledError};
use crate::group::hypercall::Hypercall;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: GPU_OPEN,
    name: "gpu_open",
    cost: None,
    handler: |request, input| {
        open_gpu(
            request,
            input.firmware,
            DEFAULT_SETTINGS,
            input.groups.gpu(),
        )
    },
}];

/// The call at a firmware whose handler no lv1 image read shows.
const UNREAD_FIRMWARE: UnmodelledError =
    UnmodelledError::new("a gpu_open at a firmware whose lv1 handler is not read");

/// The call with a `sys.boot.gos` of 3.
const BOOT_MODE_3: UnmodelledError = UnmodelledError::new(
    "a gpu_open with a sys.boot.gos of 3, which sets an lv1 GPU state word that the model does \
     not keep",
);

/// The first call with a `sys.lv1.rsxenable` below 2.
///
/// The open function fails before it stores the owner word
/// [OFW-4.93 lv1 a:0x20C594] [OFW-4.93 lv1 a:0x20C5C4]. The handler then
/// finds the first-open byte set and the owner word at 255, and goes to a
/// print path that does not return [OFW-4.93 lv1 a:0x20EB58-0x20EB74]
/// [OFW-4.93 lv1 a:0x20EC5C-0x20EC8C].
const FIRST_OPEN_DISABLED: UnmodelledError = UnmodelledError::new(
    "a first gpu_open with a sys.lv1.rsxenable below 2, which stops lv1 on its print path",
);

/// The GPU open that the partition holds.
///
/// lv1's constructor keeps r3 of the open at +8 of the GPU object
/// [OFW-4.93 lv1 a:0x20BE2C]. CellGov's hypervisor runs one partition,
/// so the state keeps no partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuOpen {
    /// The r3 of the open, which the GPU object keeps.
    pub r3: u64,
}

/// Field 1: the r3 of the open.
impl LaneValue for GpuOpen {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.r3);
    }
}

/// The values in lv1's settings table that `gpu_open` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GpuSettings {
    /// The value of `sys.boot.gos`.
    pub(crate) boot_mode: u64,
    /// The value of `sys.lv1.rsxenable`.
    pub(crate) rsx_enable: u64,
}

/// The values of lv1's default text, which the model takes as the table.
pub(crate) const DEFAULT_SETTINGS: GpuSettings = GpuSettings {
    boot_mode: DEFAULT_BOOT_MODE,
    rsx_enable: DEFAULT_RSX_ENABLE,
};

/// lv1's own r3 with r4 to r10 cleared.
fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Hypercall 210: opens the GPU for the calling partition, with the
/// settings values `settings`.
///
/// A call that fails one of lv1's checks gets lv1's own r3 and r4 to r10
/// cleared. See [`cellgov_ps3_abi::lv1::gpu`] for the checks and their
/// order. When the partition does not hold the GPU open, the call gets 0
/// in r3 and r4 to r10 cleared. The answer carries the open with r3 of
/// the call.
///
/// The model makes these assumptions:
/// - lv1's settings table holds the values of lv1's default text. This
///   is true when lv1's loader block is present, its first word is above
///   9, and its first text names none of `sys.ac.sd`,
///   `sys.lv1.rsxenable` and `sys.boot.gos`.
/// - lv1's GPU state word holds 0 or 1.
/// - The partition's word is less than 16, and its slot holds no object
///   until the first open.
/// - The allocation of the GPU object gives an object.
/// - The model holds no close. So a state with no open is a state before
///   lv1's first open, while the first-open byte is set.
///
/// lv1's first open also runs the GPU start function. The model keeps no
/// state of that function.
///
/// The call is a refusal in these cases:
/// - no lv1 image read shows the handler for `firmware`;
/// - `sys.boot.gos` is 3;
/// - `sys.lv1.rsxenable` is below 2 and the partition does not hold the
///   GPU open. lv1 does not return from this call.
pub(crate) fn open_gpu(
    request: &HypercallRequest,
    firmware: Option<&str>,
    settings: GpuSettings,
    state: &GpuState,
) -> HypercallDisposition {
    if firmware
        .filter(|fw| gpu_open_handler(fw).is_some())
        .is_none()
    {
        return UNREAD_FIRMWARE.refusal(request);
    }
    if settings.boot_mode > GPU_OPEN_BOOT_MODE_MAX {
        return lv1_answer(GPU_OPEN_BOOT_MODE_R3);
    }
    if settings.boot_mode == GPU_OPEN_BOOT_MODE_MAX {
        return BOOT_MODE_3.refusal(request);
    }
    if settings.rsx_enable < GPU_OPEN_RSX_ENABLE_MIN {
        if state.gpu_open().is_none() {
            return FIRST_OPEN_DISABLED.refusal(request);
        }
        return lv1_answer(GPU_OPEN_DISABLED_R3);
    }
    if state.gpu_open().is_some() {
        return lv1_answer(GPU_OPEN_BUSY_R3);
    }
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.push_change(GpuChange::Opened(GpuOpen { r3: request.r(3) }));
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/open_tests.rs"]
mod tests;
