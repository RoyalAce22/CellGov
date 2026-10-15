//! `sys_usbd` (syscalls 530-541): the USB host driver's guest-visible
//! vocabulary -- event codes and record sizes.
//!
//! Behaviour lives in `cellgov_lv2::host::usbd`; this module is data
//! only.
//!
//! The event codes and record sizes below are unestablished: nothing
//! in the captured evidence states them. `libusbd.sprx` is the module that would
//! witness them.

use super::errno::Lv2ErrCode;

/// The code `sys_usbd_unregister_ldd` answers for a product no register
/// recorded, as the `lv2_usbd_config_hid` capture shows. It sits
/// outside both the LV2 block at `0x8001_0001` and libusbd's own
/// `0x8011_00xx` range, and no public header names it.
pub const SYS_USBD_ERROR_PRODUCT_NOT_REGISTERED: Lv2ErrCode = Lv2ErrCode {
    code: 0x8011_1006,
    symbol: "SYS_USBD_ERROR_PRODUCT_NOT_REGISTERED",
    description: "The product string names no registered logical device driver",
};

/// `sys_usbd_receive_event` arg1: a device was attached.
pub const SYS_USBD_ATTACH: u64 = 1;

/// `sys_usbd_receive_event` arg1: a device was detached.
pub const SYS_USBD_DETACH: u64 = 2;

/// `sys_usbd_receive_event` arg1: a transfer completed.
pub const SYS_USBD_TRANSFER_COMPLETE: u64 = 3;

/// `sys_usbd_receive_event` arg1: the driver is finalizing; the
/// callback thread that receives it exits its loop.
pub const SYS_USBD_TERMINATE: u64 = 4;

/// Bytes per device record in a `sys_usbd_get_device_list` buffer.
pub const SYS_USBD_DEVICE_RECORD_LEN: usize = 4;

/// Transfer slots the driver keeps per handle.
pub const SYS_USBD_MAX_TRANSFERS: u32 = 0x44;
