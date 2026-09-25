//! The debug observers a boot installs for the program driving it, and
//! the three watches CellGov ships as such observers.
//!
//! The boot asks a [`DebugTaps`] for its observers, hands them to every
//! PPU unit and to the runtime, and reports each firmware set it binds.
//! [`WatchTaps`] installs the HLE return watch, the store watch and the
//! value sample behind it. Each watch writes a binary capture of its
//! own, through a [`RecordFile`]:
//!
//! - [`hle_watch`]: `CGHW` version 1;
//! - [`store_watch`]: `CGSW` version 1;
//! - [`value_sample`]: `CGVS` version 2.
//!
//! No reader for any of the three ships in this repository. A write the
//! host refuses ends that capture, and the watch hands the refusal to
//! its owner rather than printing it.
//!
//! [`StateHashCensus`] is a PPU observer that writes no capture: it
//! counts state-hash collisions over the states a run passes through.
//! [`WithPpuTap`] installs it beside the watches.

mod debug_taps;
pub mod hle_watch;
mod record_file;
pub mod state_hash_census;
pub mod store_watch;
pub mod value_sample;
mod watch_taps;

pub use debug_taps::{DebugTaps, NoTaps};
pub use hle_watch::{HleWatch, HleWatchSpec};
pub use record_file::RecordFile;
pub use state_hash_census::{CensusReport, StateHashCensus, WithPpuTap};
pub use store_watch::{StoreWatch, StoreWatchSpec};
pub use value_sample::{ValueSample, ValueSampleSpec};
pub use watch_taps::{WatchEvent, WatchKind, WatchReporter, WatchTaps};
