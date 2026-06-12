//! [`PpuDecodeError`] -- raised by the decoder when a 32-bit word
//! either has no decoder arm or matches no documented Cell BE PPU
//! encoding.
//!
//! Two failure modes are distinguished at the rejection site so the
//! reader can act on each: hits in the spec-citation directory in
//! [`super::known_encodings`] produce
//! [`PpuDecodeError::DecoderArmUnimplemented`] with the canonical
//! mnemonic and a [`Locator`] discriminating between an opcode gap
//! and an SPR / TBR operand gap; both-tables-miss produces
//! [`PpuDecodeError::EncodingNotRecognized`] carrying the raw word
//! only, because naming a non-existent instruction would be a lie.

/// What was looked up to produce a
/// [`PpuDecodeError::DecoderArmUnimplemented`].
///
/// Discriminates Table 1 (opcode gap, keyed `(primary, xo)`) from
/// Table 2 (SPR / TBR operand gap, keyed on the post-half-swap
/// register number and tagged with the XFX direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locator {
    /// The encoding's `(primary, xo)` pair has no decoder arm.
    Opcode {
        /// Primary opcode (top 6 bits).
        primary: u8,
        /// Extended opcode (form-dependent width, packed into 16
        /// bits).
        xo: u16,
    },
    /// `mfspr` / `mftb` / `mtspr` decoded; the named register
    /// number after the XFX half-swap has no decoder handling.
    Spr {
        /// Direction-tagged opcode name: `"mfspr"`, `"mftb"`, or
        /// `"mtspr"`.
        op_mnemonic: &'static str,
        /// 10-bit SPR / TBR number after the half-swap is undone.
        spr: u16,
    },
}

/// Why decoding failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PpuDecodeError {
    /// The encoding (or SPR / TBR selector) is documented per the
    /// `known_encodings` directory but has no decoder
    /// implementation.
    ///
    /// Display reads `missing <mnemonic> (primary <p>, xo <x>)`
    /// for opcode gaps and
    /// `missing <mnemonic> (<op_mnemonic>, SPR <n>)` for SPR /
    /// TBR gaps. No spec citation is appended.
    #[error("{}", display_arm_unimplemented(*locator, mnemonic))]
    DecoderArmUnimplemented {
        /// What the lookup returned.
        locator: Locator,
        /// Canonical mnemonic the spec defines for the looked-up
        /// key.
        mnemonic: &'static str,
        /// The raw 32-bit word the rejection fired on.
        raw: u32,
    },
    /// Neither Table 1 nor Table 2 matched. The encoding may be
    /// garbage, a mis-aligned execution point, or a Cell encoding
    /// the spec corpus omits; the rejection carries the raw word
    /// only.
    #[error("no documented encoding for raw 0x{raw:08x}")]
    EncodingNotRecognized {
        /// The raw 32-bit word the rejection fired on.
        raw: u32,
    },
}

impl PpuDecodeError {
    /// The raw 32-bit word that triggered the rejection.
    pub fn raw(&self) -> u32 {
        match self {
            Self::DecoderArmUnimplemented { raw, .. } | Self::EncodingNotRecognized { raw, .. } => {
                *raw
            }
        }
    }
}

/// Render the Display body for
/// [`PpuDecodeError::DecoderArmUnimplemented`]. Kept as a free
/// function so the `#[error("{}", ...)]` attribute can call it
/// without a turbofish on the closure argument.
fn display_arm_unimplemented(locator: Locator, mnemonic: &str) -> String {
    match locator {
        Locator::Opcode { primary, xo } => {
            format!("missing {mnemonic} (primary {primary}, xo {xo})")
        }
        Locator::Spr { op_mnemonic, spr } => {
            format!("missing {mnemonic} ({op_mnemonic}, SPR {spr})")
        }
    }
}

#[cfg(test)]
#[path = "tests/decode_error_tests.rs"]
mod tests;
