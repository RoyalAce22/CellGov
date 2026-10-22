//! Canonical 32-bit word for a decoded [`PpuInstruction`].
//!
//! [`encode`] is total over the variants the decoder produces, so
//! `decode(encode(i)) == Ok(i)` for every decoded `i`, and
//! `encode(decode(w))` is `w` with the bits in [`reserved_bits`]
//! cleared unless [`alias`](fn@alias) names `w` as a second spelling. A
//! variant with no arm here fails to compile.
// [PPC-Book1 p:7 s:1.7 Instruction formats] OPCD at bits 0:5; XO is form-dependent.

mod alias;
mod fields;
mod reserved;
mod word;

#[cfg(doc)]
use super::PpuInstruction;

pub use alias::{alias, Alias};
pub use reserved::reserved_bits;
pub use word::{encode, EncodeError};

#[cfg(test)]
#[path = "tests/encode_tests.rs"]
mod tests;
