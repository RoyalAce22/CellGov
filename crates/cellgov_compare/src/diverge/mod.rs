//! Trace divergence scanner and zoom-lookup.
//!
//! [`diverge`] is the streaming scanner over `PpuStateHash` records;
//! [`zoom_lookup`] is the linear lookup into `PpuStateFull` snapshots
//! for register-level investigation once a divergence step is known.

mod scan;
mod zoom;

pub use scan::{diverge, DivergeField, DivergeReport};
pub use zoom::{zoom_lookup, RegDiff, ZoomLookup};
