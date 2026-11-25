//! The storage device hypercalls: flash, HDD and Blu-ray storage.
//!
//! The group answers `storage_write` (hypercall 246) to the notification
//! device, device -1 of the storage bus. It keeps the notification
//! requests that lv1 holds for the partition, and the event mask of the
//! partition's storage bus open.

mod registration;
mod write;

pub(crate) use registration::GROUP;
pub use registration::{NotifyRequest, StorageChange, StorageState};
