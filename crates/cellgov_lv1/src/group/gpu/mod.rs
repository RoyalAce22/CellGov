//! The GPU hypercalls.
//!
//! The group answers `gpu_open` at 4.93.
//!
//! [`GpuState`] holds the state of the group.

mod open;
mod registration;

pub use open::GpuOpen;
pub(crate) use registration::GROUP;
pub use registration::{GpuChange, GpuState};
