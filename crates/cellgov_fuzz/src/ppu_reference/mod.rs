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

#[cfg(test)]
#[path = "../tests/ppu_reference_capture.rs"]
mod capture;

#[cfg(test)]
#[path = "../tests/ppu_reference_floor.rs"]
mod floor;

#[cfg(test)]
#[path = "../tests/ppu_reference_documented.rs"]
mod documented;

#[cfg(test)]
#[path = "../tests/ppu_reference_documented_tests.rs"]
mod documented_tests;

#[cfg(test)]
#[path = "../tests/ppu_reference_capture_tests.rs"]
mod capture_tests;
