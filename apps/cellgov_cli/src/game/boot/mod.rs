//! Boot preparation shared between `boot run` and `boot bench`.
//!
//! One module per stage of [`prepare()`], which owns the order they
//! run in.

mod entry;
mod finish;
mod firmware;
mod host;
mod image;
mod loaders;
mod module_start;
mod params;
mod prepare;
mod providers;
mod types;

pub use image::HLE_HEAP_BASE;
pub(super) use prepare::prepare;
pub(super) use types::{PrepareOptions, PreparedBoot};
