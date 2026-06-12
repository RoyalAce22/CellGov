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

/// Table 3a: documented primary-4 VA-form XOs (6-bit, in 0x20..=0x2F).
///
/// The decoder previously fabricated `Va { xo }` for any 6-bit XO in
/// 0x20..=0x2F including ones the AltiVec PEM does not define. This
/// directory is the gate: a primary-4 word whose `xo_6` is not in
/// this list rejects via `reject_opcode` instead of silently
/// becoming an opaque `Va { xo }` the executor would later have to
/// fault on.
///
/// Source: AltiVec-PEM Appendix A.5 Table A-5, transcribed via
/// `scripts/gen_altivec_comp_table.py`. The list includes vsldoi
/// (XO 44) and vsel/vperm/vmsum*/vmhaddshs etc. (XOs 32-47 minus 35
/// and 45 which are reserved).
///
/// MUST stay sorted ascending; [`is_known_va`] is a binary search.
pub const KNOWN_VA_XOS: &[u8] = &[32, 33, 34, 36, 37, 38, 39, 40, 41, 42, 43, 44, 46, 47];

/// Table 3b: documented primary-4 VX-form / VXR-form XOs.
///
/// The decoder dispatches primary 4 by `xo_11 = raw & 0x7FF`. VX-form
/// uses the full 11 bits; VXR-form (compare ops) uses bit 21 of raw
/// (which is bit 10 of `xo_11`) as the record bit, with the 10-bit
/// XO at raw bits 22..31. Both VXR Rc=0 (base XO) and VXR Rc=1
/// (`1024 | base`) forms are valid and must appear here.
///
/// Source: AltiVec-PEM Appendix A.5 Table A-6 / A-7, transcribed via
/// `scripts/gen_altivec_comp_table.py`. Includes 109 VX, 13 VXR Rc=0,
/// 13 VXR Rc=1, plus vxor at XO 1220 (which the decoder routes to
/// the typed `Vxor` variant before this lookup runs).
///
/// MUST stay sorted ascending; [`is_known_vx`] is a binary search.
pub const KNOWN_VX_XOS: &[u16] = &[
    0, 2, 4, 6, 8, 10, 12, 14, 64, 66, 68, 70, 72, 74, 76, 78, 128, 130, 132, 134, 140, 142, 198,
    206, 258, 260, 264, 266, 268, 270, 322, 324, 328, 330, 332, 334, 384, 386, 388, 394, 396, 398,
    452, 454, 458, 462, 512, 514, 516, 518, 520, 522, 524, 526, 576, 578, 580, 582, 584, 586, 588,
    590, 640, 642, 644, 646, 650, 652, 654, 708, 710, 714, 718, 768, 770, 772, 774, 776, 778, 780,
    782, 832, 834, 836, 838, 840, 842, 844, 846, 896, 898, 900, 902, 906, 908, 966, 970, 974, 1024,
    1026, 1028, 1030, 1034, 1036, 1088, 1090, 1092, 1094, 1098, 1100, 1152, 1154, 1156, 1158, 1220,
    1222, 1282, 1284, 1346, 1408, 1410, 1478, 1536, 1542, 1544, 1600, 1606, 1608, 1664, 1670, 1672,
    1734, 1792, 1798, 1800, 1856, 1862, 1920, 1926, 1928, 1990,
];

/// O(log N) lookup over [`KNOWN_VA_XOS`].
pub fn is_known_va(xo: u8) -> bool {
    KNOWN_VA_XOS.binary_search(&xo).is_ok()
}

/// O(log N) lookup over [`KNOWN_VX_XOS`].
pub fn is_known_vx(xo: u16) -> bool {
    KNOWN_VX_XOS.binary_search(&xo).is_ok()
}

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
