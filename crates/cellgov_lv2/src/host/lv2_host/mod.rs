//! `Lv2Host` model, its `FirmwareIdentity` payload, and the state
//! primitives exposed to the dispatch submodules.

mod accessors;
mod children;
mod counters;
mod mmapper;
mod model;
mod process;
mod seeds;

pub use model::{FirmwareIdentity, Lv2Host};

#[cfg(test)]
#[path = "tests/lv2_host_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/child_alias_tests.rs"]
mod child_alias_tests;
