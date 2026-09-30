//! Offline SPU vectors and operator-supplied hardware observations.

mod campaign;
mod compare;
mod set_compare;
mod set_convert;
mod set_replay;
mod set_types;
mod set_validate;
mod types;
mod validate;

pub use campaign::*;
pub use compare::*;
pub use set_compare::*;
pub use set_replay::*;
pub use set_types::*;
pub use set_validate::*;
pub use types::*;
pub use validate::*;

#[cfg(test)]
#[path = "../tests/spu_reference_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/spu_reference_tag_tests.rs"]
mod tag_tests;

#[cfg(test)]
#[path = "../tests/spu_reference_set_tests.rs"]
mod set_tests;

#[cfg(test)]
#[path = "../tests/spu_reference_campaign_tests.rs"]
mod campaign_tests;
