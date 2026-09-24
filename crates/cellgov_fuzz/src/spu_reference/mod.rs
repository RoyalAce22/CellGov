//! Offline SPU vectors and operator-supplied hardware observations.

mod compare;
mod types;
mod validate;

pub use compare::*;
pub use types::*;
pub use validate::*;

#[cfg(test)]
#[path = "../tests/spu_reference_tests.rs"]
mod tests;
