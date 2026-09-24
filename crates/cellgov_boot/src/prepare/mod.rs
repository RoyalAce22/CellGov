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
mod providers;
mod stages;
mod types;

pub use entry::EntryError;
pub use finish::PatchError;
pub use host::HostBindError;
pub use image::{ImageError, HLE_HEAP_BASE};
pub use loaders::spu_unit;
pub use params::ParamsError;
pub use providers::ProviderError;
pub use stages::prepare;
pub use types::{
    AuthorityIdSource, BootServices, DiagnosticOptions, ExecutionOptions, PrepareOptions,
    PreparedBoot, StartupTimings, StrictReservedConflict, TitleOptions,
};
