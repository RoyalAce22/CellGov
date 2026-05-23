//! Typed PPU instruction forms.
//!
//! Variants carry decoded register indices, immediates, and flags.
//! Decode produces these; execute consumes them. Unknown encodings
//! decode to `PpuDecodeError::Unsupported` rather than a variant.

mod decode_error;
mod insn;

pub use decode_error::PpuDecodeError;
pub use insn::PpuInstruction;
