//! Microtest manifest parsing.
//!
//! A manifest is a TOML file that ties a CellGov scenario to an RPCS3
//! test binary, declares memory regions to observe, and specifies the
//! expected outcome. One manifest per microtest. The schema is
//! decomposed by axis (field-level value types, section structs,
//! loaders) across submodules; the public surface is re-exported
//! below.

mod fields;
mod loader;
mod model;
mod sections;

pub use fields::{DecoderField, MemoryRegionSpec, OutcomeField};
pub use loader::{load, parse, ManifestError};
pub use model::Manifest;
pub use sections::{CellGovSection, ExpectSection, ObserveSection, Rpcs3Section, TestSection};
