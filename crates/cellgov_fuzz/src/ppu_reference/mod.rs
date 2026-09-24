//! Versioned offline references for PPU execution observations.

mod compare;
mod types;
mod validate;

pub use crate::reference::ReferenceField;

pub use compare::*;
pub use types::*;
pub use validate::*;

#[cfg(test)]
#[path = "../tests/ppu_reference_tests.rs"]
mod tests;
