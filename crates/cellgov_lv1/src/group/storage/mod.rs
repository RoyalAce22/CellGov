//! The storage device hypercalls: flash, HDD and Blu-ray storage.
//!
//! The group answers two hypercalls to the notification device, device -1
//! of the storage bus:
//! - `storage_write` (hypercall 246), which makes a request;
//! - `storage_get_async_status` (hypercall 249), which gives the
//!   partition a completed request.
//!
//! It keeps these values:
//! - the notification requests that lv1 holds for the partition;
//! - the event mask of the partition's storage bus open;
//! - the storage devices that lv1 registered for the partition, whose
//!   repository nodes [`device_registration`] answers.

pub(crate) mod device_registration;
mod registration;
mod status;
mod write;

pub(crate) use registration::GROUP;
pub use registration::{NotifyRequest, StorageChange, StorageState};
