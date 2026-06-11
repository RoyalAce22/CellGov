//! Typed PPU instruction forms.
//!
//! Variants carry decoded register indices, immediates, and flags.
//! Decode produces these; execute consumes them. Encodings the
//! decoder rejects resolve through [`known_encodings`] to a
//! spec-named [`PpuDecodeError::DecoderArmUnimplemented`] or to
//! [`PpuDecodeError::EncodingNotRecognized`].

mod decode_error;
mod insn;
pub mod known_encodings;

pub use decode_error::{Locator, PpuDecodeError};
pub use insn::PpuInstruction;
pub use known_encodings::{OpcodeGap, SprDirection, SprGap};
