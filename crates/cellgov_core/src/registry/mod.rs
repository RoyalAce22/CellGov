//! Unit registry: assigns stable `UnitId`s, stores units behind an
//! object-safe trait, iterates in id order.

mod access;
mod hash;
mod overrides;
mod pending;
mod registration;
mod state;
mod unit_trait;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub use state::UnitRegistry;
pub use unit_trait::RegisteredUnit;
