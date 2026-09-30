//! Trace divergence scanner and zoom-lookup.
//!
//! [`diverge`] is the streaming scanner over the per-step hash
//! records; [`zoom_lookup`] and [`spu_zoom_lookup`] are the linear
//! lookups into the full-state snapshots for register-level
//! investigation once a divergence step is known.
//!
//! [Wang2024 p:340:17 s:3.9] Compare a hash of the state first; when
//! the hashes differ, run again with the full state exposed to see
//! which value differs.

mod scan;
mod zoom;

pub use scan::{
    diverge, trace_scheme, DivergeField, DivergeReport, StateHashKind, StateStream, TraceSchemes,
};
pub use zoom::{
    spu_zoom_lookup, zoom_lookup, RegDiff, SpuField, SpuRegDiff, SpuZoomLookup, ZoomLookup,
};
