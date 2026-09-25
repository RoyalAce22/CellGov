//! Trace divergence scanner and zoom-lookup.
//!
//! [`diverge`] is the streaming scanner over `PpuStateHash` records;
//! [`zoom_lookup`] is the linear lookup into `PpuStateFull` snapshots
//! for register-level investigation once a divergence step is known.
//!
//! [Wang2024 p:340:17 s:3.9] Compare a hash of the state first; when
//! the hashes differ, run again with the full state exposed to see
//! which value differs.

mod scan;
mod zoom;

pub use scan::{diverge, trace_scheme, DivergeField, DivergeReport, TraceSchemes};
pub use zoom::{zoom_lookup, RegDiff, ZoomLookup};
