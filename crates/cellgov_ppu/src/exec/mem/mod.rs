//! Memory dispatch: integer / atomic / vector / floating-point loads
//! and stores, plus `dcbz`. The scalar loads and stores decode to a
//! [`Scalar`] and run through one [`load`] and one [`store`]. Every
//! path shares the `load_ze` / `load_se` / `buffer_store` helpers and
//! the `LoadPort` from `memory_helpers`, so the reservation
//! clear-sweep and the read intent stay consistent across them.
//!
//! [`Scalar`]: scalar::Scalar
//! [`load`]: scalar::load
//! [`store`]: scalar::store

mod execute;
mod helpers;
mod scalar;
mod string;

pub(crate) use execute::execute;

#[cfg(test)]
#[path = "tests/mem_tests.rs"]
mod tests;
