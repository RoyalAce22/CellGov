//! Versioned offline references for PPU execution observations.

mod campaign;
mod compare;
mod set;
mod types;
mod units;
mod validate;

pub use crate::reference::ReferenceField;

pub use campaign::*;
pub use compare::*;
pub use set::*;
pub use types::*;
pub use units::*;
pub use validate::*;

#[cfg(test)]
#[path = "../tests/ppu_reference_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/ppu_reference_campaign_tests.rs"]
mod campaign_tests;
