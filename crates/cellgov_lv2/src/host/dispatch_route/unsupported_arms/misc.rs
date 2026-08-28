//! Unsupported arms whose whole behaviour is a single answer.

use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_tty_read` (402): returns EIO (matches retail LV2 outside
    /// debug-console mode).
    pub(in crate::host::dispatch_route) fn dispatch_tty_read(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_EIO.into())
    }

    /// DEX-only slot (462): retail liblv2 takes its fallback path on
    /// ENOSYS.
    pub(in crate::host::dispatch_route) fn dispatch_uns_func_462(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `sys_hid_manager_is_process_permission_root` (512): returns 0
    /// (retail titles run unprivileged).
    pub(in crate::host::dispatch_route) fn dispatch_hid_is_root(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_gamepad_ycon_if` (621): stub returning CELL_OK.
    pub(in crate::host::dispatch_route) fn dispatch_gamepad_ycon_if(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.gamepad_ycon_if_stub",
            format_args!(
                "sys_gamepad_ycon_if: stub returning CELL_OK; matches RPCS3's \
                 todo-and-OK stub"
            ),
        );
        Lv2Dispatch::immediate(0)
    }

    /// `sys_rsx_attribute` (677): returns CELL_OK without state change.
    pub(in crate::host::dispatch_route) fn dispatch_rsx_attribute(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.rsx_attribute_stub",
            format_args!("sys_rsx_attribute: stub returning CELL_OK with no state change"),
        );
        Lv2Dispatch::immediate(0)
    }
}
