//! Spec-citation directory for decoder fall-through diagnostics.
//!
//! [`crate::instruction::PpuDecodeError`] distinguishes "documented
//! PPU instruction this decoder does not yet implement" from "no
//! documented instruction matches". Telling those apart at the
//! rejection site requires knowing *which* documented instruction a
//! `(primary, xo)` pair names; this module is that answer: sorted
//! const directories of named-but-unimplemented encodings,
//! transcribed once against PPC v2.02 + AltiVec PEM + CBE Handbook.
//! The directories only shrink, as instructions land in the decoder.
//!
//! - [`OPCODE_GAPS`] is keyed `(primary, xo)` for instructions
//!   whose 32-bit encoding has no decoder arm.
//! - [`MFSPR_GAPS`] / [`MFTB_GAPS`] / [`MTSPR_GAPS`] are keyed on
//!   the post-half-swap SPR / TBR number; the XFX opcodes themselves
//!   decode, only specific register selectors are unwired. Three
//!   sub-tables resolve the read-vs-write ambiguity (SPR 1 is
//!   `mfxer` under [`MFSPR_GAPS`], `mtxer` under [`MTSPR_GAPS`]).
//!
//! Lookups are binary searches over sorted const slices -- no heap
//! traffic on the hot decode-error path. A `None` from
//! [`opcode_gap`] / [`spr_gap`] is itself the diagnostic for
//! `EncodingNotRecognized`. The sort-order and table-vs-decoder
//! disjointness invariants are test-asserted.

/// A documented encoding the decoder cannot yet turn into a
/// [`crate::instruction::PpuInstruction`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeGap {
    /// Primary opcode (top 6 bits of the encoding).
    pub primary: u8,
    /// Extended opcode value. The width depends on form (10-bit
    /// for X-form, 9-bit for XO-form, 3-bit for MD-form sub-XO,
    /// etc.); the directory treats it as a 16-bit unsigned key.
    pub xo: u16,
    /// Canonical mnemonic the spec defines for this encoding.
    pub mnemonic: &'static str,
}

/// A documented SPR / TBR selector the matching `mfspr` / `mftb` /
/// `mtspr` arm cannot yet route. Returned by [`spr_gap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SprGap {
    /// 10-bit SPR / TBR number after the XFX half-swap.
    pub spr: u16,
    /// Canonical mnemonic for the matching read or write.
    pub mnemonic: &'static str,
}

/// Sort key for [`OPCODE_GAPS`]: `(primary, xo)` lexicographic.
const fn opcode_key(primary: u8, xo: u16) -> u32 {
    ((primary as u32) << 16) | (xo as u32)
}

/// Resolve a `(primary, xo)` to its canonical mnemonic, or `None`
/// if the spec corpus does not define the encoding (the rejection
/// site emits `EncodingNotRecognized`).
///
/// O(log N) over [`OPCODE_GAPS`] with no allocation. Intended for
/// the decoder's fall-through arms and the prescan accumulator.
pub fn opcode_gap(primary: u8, xo: u16) -> Option<OpcodeGap> {
    let k = opcode_key(primary, xo);
    OPCODE_GAPS
        .binary_search_by_key(&k, |row| opcode_key(row.primary, row.xo))
        .ok()
        .map(|i| OPCODE_GAPS[i])
}

/// XFX direction tag for [`spr_gap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprDirection {
    /// `mfspr` at primary 31 / XO 339 (read SPR -> RT).
    MfSpr,
    /// `mftb`  at primary 31 / XO 371 (read TBR -> RT).
    MfTb,
    /// `mtspr` at primary 31 / XO 467 (write SPR <- RS).
    MtSpr,
}

impl SprDirection {
    /// Direction-tagged opcode name used in the error display.
    pub fn op_mnemonic(self) -> &'static str {
        match self {
            Self::MfSpr => "mfspr",
            Self::MfTb => "mftb",
            Self::MtSpr => "mtspr",
        }
    }
}

/// Resolve an `(SprDirection, spr)` to its canonical mnemonic.
///
/// `None` if the direction's sub-table doesn't list this SPR / TBR
/// (the rejection site emits `EncodingNotRecognized`).
pub fn spr_gap(direction: SprDirection, spr: u16) -> Option<SprGap> {
    let table: &[SprGap] = match direction {
        SprDirection::MfSpr => MFSPR_GAPS,
        SprDirection::MfTb => MFTB_GAPS,
        SprDirection::MtSpr => MTSPR_GAPS,
    };
    table
        .binary_search_by_key(&spr, |row| row.spr)
        .ok()
        .map(|i| table[i])
}

/// Table 1: documented `(primary, xo)` encodings the decoder
/// cannot yet handle.
///
/// MUST stay sorted by `(primary, xo)`; [`opcode_gap`] is a binary
/// search.
pub const OPCODE_GAPS: &[OpcodeGap] = &[];

// The primary-4 VX / VA gate directories formerly here
// (`KNOWN_VX_XOS` / `KNOWN_VA_XOS`) are retired: the decoder now
// gates on [`crate::instruction::ops::VxOp`] /
// [`crate::instruction::ops::VaOp`], whose discriminants carry the
// same transcription with compiler-enforced consumer coverage.

/// Table 2 (`mfspr` direction). SPRs CellGov does not yet implement
/// the read for. The XFX opcode itself is decoded (XO 339); the
/// SPR selector is what's missing.
///
/// MUST stay sorted by `spr`; [`spr_gap`] is a binary search.
pub const MFSPR_GAPS: &[SprGap] = &[
    SprGap {
        spr: 18,
        mnemonic: "mfdsisr",
    },
    SprGap {
        spr: 19,
        mnemonic: "mfdar",
    },
    SprGap {
        spr: 22,
        mnemonic: "mfdec",
    },
    SprGap {
        spr: 26,
        mnemonic: "mfsrr0",
    },
    SprGap {
        spr: 27,
        mnemonic: "mfsrr1",
    },
    SprGap {
        spr: 272,
        mnemonic: "mfsprg0",
    },
    SprGap {
        spr: 273,
        mnemonic: "mfsprg1",
    },
    SprGap {
        spr: 274,
        mnemonic: "mfsprg2",
    },
    SprGap {
        spr: 275,
        mnemonic: "mfsprg3",
    },
    SprGap {
        spr: 280,
        mnemonic: "mfasr",
    },
    SprGap {
        spr: 282,
        mnemonic: "mfear",
    },
    SprGap {
        spr: 287,
        mnemonic: "mfpvr",
    },
];

/// Table 2 (`mftb` direction). Currently empty: the only defined
/// TBRs (268 TB, 269 TBU) are already implemented; any other
/// selector is genuinely undocumented and surfaces as
/// `EncodingNotRecognized`.
pub const MFTB_GAPS: &[SprGap] = &[];

/// Table 2 (`mtspr` direction). SPRs CellGov does not yet
/// implement the write for. Mirrors [`MFSPR_GAPS`]'s structure;
/// the entries differ because some SPRs are read-only.
///
/// MUST stay sorted by `spr`; [`spr_gap`] is a binary search.
pub const MTSPR_GAPS: &[SprGap] = &[
    SprGap {
        spr: 18,
        mnemonic: "mtdsisr",
    },
    SprGap {
        spr: 19,
        mnemonic: "mtdar",
    },
    SprGap {
        spr: 22,
        mnemonic: "mtdec",
    },
    SprGap {
        spr: 26,
        mnemonic: "mtsrr0",
    },
    SprGap {
        spr: 27,
        mnemonic: "mtsrr1",
    },
    SprGap {
        spr: 272,
        mnemonic: "mtsprg0",
    },
    SprGap {
        spr: 273,
        mnemonic: "mtsprg1",
    },
    SprGap {
        spr: 274,
        mnemonic: "mtsprg2",
    },
    SprGap {
        spr: 275,
        mnemonic: "mtsprg3",
    },
    SprGap {
        spr: 282,
        mnemonic: "mtear",
    },
];

#[cfg(test)]
#[path = "tests/known_encodings_tests.rs"]
mod tests;
