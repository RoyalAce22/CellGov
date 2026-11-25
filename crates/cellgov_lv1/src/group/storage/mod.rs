//! The storage device hypercalls: flash, HDD and Blu-ray storage.
//!
//! The group answers three hypercalls to the notification device, device
//! -1 of the storage bus:
//! - `storage_read` (hypercall 245), which reports a storage device;
//! - `storage_write` (hypercall 246), which makes a request;
//! - `storage_get_async_status` (hypercall 249), which gives the
//!   partition a completed request.
//!
//! It also answers `storage_send_device_command` (hypercall 248), which
//! starts a command on a storage device.
//!
//! It keeps these values:
//! - the notification requests that lv1 holds for the partition;
//! - the event mask of the partition's storage bus open;
//! - the storage devices that lv1 registered for the partition, whose
//!   repository nodes [`device_registration`] answers;
//! - the number of storage devices that lv1 reported to the partition;
//! - the notification read that lv1 holds waiting for the partition;
//! - the device commands that lv1 holds for the partition.

mod command;
pub(crate) mod device_registration;
mod read;
mod registration;
mod status;
mod write;

pub(crate) use registration::GROUP;
pub use registration::{DeviceCommand, NotifyRequest, StorageChange, StorageState, WaitingRead};
