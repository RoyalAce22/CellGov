//! `sys_usbd` (syscalls 530-541): the USB host driver's guest-visible
//! vocabulary -- event codes and record sizes.
//!
//! Behaviour lives in `cellgov_lv2::host::usbd`; this module is data
//! only.
//!
//! The event codes and record sizes below are unestablished: nothing
//! in the captured evidence states them. `libusbd.sprx` is the module that would
//! witness them.

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
