//! Spec-citation directory for decoder fall-through diagnostics.
//!
//! The PPU decoder rejects any 32-bit word it cannot turn into a
//! [`crate::instruction::PpuInstruction`] variant.
//! [`crate::instruction::PpuDecodeError`] distinguishes two
//! rejection modes -- "the encoding is a documented PPU instruction
//! that this decoder does not yet implement" and "the encoding does
//! not match any documented instruction" -- because conflating
//! them turns a fixable gap into a generic deep-commit fault.
//! Telling those modes apart at the rejection site requires
//! knowing *which* documented instruction a `(primary, xo)` pair
//! names; the decoder source itself can't answer that, because the
//! whole point is that the decoder has no arm for it.
//!
//! This module is that answer: two hand-written sorted const
//! directories of named-but-unimplemented encodings.
//!
//! - [`OPCODE_GAPS`] (Table 1) is keyed `(primary, xo)` for
//!   instructions whose 32-bit encoding has no decoder arm:
//!   `lvsr` (primary 31, XO 38), the AltiVec-memory family, the
//!   scalar gaps documented in the audit (popcntb, the
//!   byte-reverse family, load/store-with-update, etc.).
//! - [`MFSPR_GAPS`] / [`MFTB_GAPS`] / [`MTSPR_GAPS`] (Table 2,
//!   three sub-tables) are keyed on the post-half-swap SPR / TBR
//!   number. The XFX opcodes themselves are decoded (`mfspr` at
//!   XO 339, `mftb` at 371, `mtspr` at 467); only specific
//!   register selectors beyond LR / CTR / TB / TBU are documented
//!   but not yet wired. The three sub-tables resolve the
//!   read-vs-write ambiguity: SPR 1 looks up as `mfxer` under
//!   [`MFSPR_GAPS`] and as `mtxer` under [`MTSPR_GAPS`].
//!
//! Why two frozen const directories instead of one runtime-loaded
//! table:
//!
//! - The Cell BE PPU ISA is a closed, historical instruction set;
//!   no upstream is adding instructions and no one is removing
//!   them. The named-but-unimplemented universe is finite,
//!   knowable, and writable once. Both directories are
//!   transcribed against PPC v2.02 + AltiVec PEM + CBE Handbook
//!   once, verified, and frozen. The directories never grow under
//!   normal use; they only shrink as instructions land in the
//!   decoder.
//! - A binary search over a sorted const slice costs about log2(N)
//!   comparisons with no heap traffic, sized into the binary at
//!   compile time. The Display strings in [`super::PpuDecodeError`]
//!   format without allocation, so a runtime decode error in the
//!   hot interpreter loop costs no extra syscalls.
//! - A `None` from [`opcode_gap`] / [`spr_gap`] is itself the
//!   diagnostic for `EncodingNotRecognized` -- the rejected raw
//!   word is either garbage, mis-aligned execution, or addresses
//!   a CellGov-out-of-scope SPR.
//!
//! Adding a row to either directory means transcribing the spec
//! encoding into [`OPCODE_GAPS`] / [`MFSPR_GAPS`] / etc., keeping
//! the sort order. The sort-order invariant is asserted by a
//! `#[cfg(test)]` check, and the table-vs-decoder disjointness
//! invariant ("no row whose encoding the decoder actually
//! handles") is asserted by a separate form-aware test in
//! `crate::decode`. Removing a row -- which is the only change
//! that should happen during ordinary Phase 40 Layer 3 work --
//! requires deleting the matching arm-add commit's row.

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
mod tests {
    use super::*;

    #[test]
    fn opcode_gaps_sorted_for_binary_search() {
        for window in OPCODE_GAPS.windows(2) {
            let lo = opcode_key(window[0].primary, window[0].xo);
            let hi = opcode_key(window[1].primary, window[1].xo);
            assert!(
                lo < hi,
                "OPCODE_GAPS must be sorted by (primary, xo); offenders: {:?} >= {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn spr_gap_sub_tables_sorted_for_binary_search() {
        for (name, table) in [
            ("MFSPR_GAPS", MFSPR_GAPS),
            ("MFTB_GAPS", MFTB_GAPS),
            ("MTSPR_GAPS", MTSPR_GAPS),
        ] {
            for window in table.windows(2) {
                assert!(
                    window[0].spr < window[1].spr,
                    "{name} must be sorted by spr; offenders: {:?} >= {:?}",
                    window[0],
                    window[1]
                );
            }
        }
    }

    #[test]
    fn known_va_xos_sorted_for_binary_search() {
        for window in KNOWN_VA_XOS.windows(2) {
            assert!(
                window[0] < window[1],
                "KNOWN_VA_XOS must be sorted ascending; offenders: {} >= {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn known_vx_xos_sorted_for_binary_search() {
        for window in KNOWN_VX_XOS.windows(2) {
            assert!(
                window[0] < window[1],
                "KNOWN_VX_XOS must be sorted ascending; offenders: {} >= {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn known_va_directory_covers_vsldoi_and_vsel() {
        // Spot-check anchors: vsldoi (XO 44, the only typed VA arm)
        // and vsel (XO 42, a representative VA stub).
        assert!(is_known_va(44));
        assert!(is_known_va(42));
        // 35 and 45 are reserved within 0x20..=0x2F and must not appear.
        assert!(!is_known_va(35));
        assert!(!is_known_va(45));
    }

    #[test]
    fn known_vx_directory_covers_typed_and_stub_anchors() {
        // Vxor (typed) at 1220; vmaxub (stub) at 2; vcmpequb VXR
        // Rc=0 at 6, Rc=1 at 1030.
        assert!(is_known_vx(1220));
        assert!(is_known_vx(2));
        assert!(is_known_vx(6));
        assert!(is_known_vx(1030));
        // 1 / 3 / 5 / 7 / 9 / 11 / 13 / 15 are odd VX positions not
        // assigned to any AltiVec-PEM instruction.
        assert!(!is_known_vx(1));
        assert!(!is_known_vx(3));
    }

    #[test]
    fn altivec_memory_family_no_longer_in_directory() {
        // Stage 40E landed Lvsl / Lvebx / Lvsr / Lvehx / Lvewx /
        // Stvebx / Stvehx / Stvewx / Lvxl / Stvxl as PpuInstruction
        // variants. The directory shrinks; the form-aware
        // disjointness test in `decode` enforces that rows whose
        // arm now decodes must be absent from OPCODE_GAPS.
        for xo in [6u16, 7, 38, 39, 71, 135, 167, 199, 359, 487] {
            assert!(
                opcode_gap(31, xo).is_none(),
                "AltiVec-memory XO {xo} still present in OPCODE_GAPS"
            );
        }
    }

    #[test]
    fn cbe_unaligned_family_no_longer_in_directory() {
        // Stage 40C.4 graduated lvlxl / lvrxl / stvlx / stvrx /
        // stvlxl / stvrxl into the decoder. The two LRU-hint loads
        // share semantics with Lvlx / Lvrx; the four stores are
        // harness-gated fault stubs until Stage 40D verifies them.
        for xo in [647u16, 711, 775, 839, 903, 967] {
            assert!(
                opcode_gap(31, xo).is_none(),
                "CBE-unaligned XO {xo} still present in OPCODE_GAPS"
            );
        }
    }

    #[test]
    fn xer_no_longer_in_spr_gaps() {
        // Stage 40C.10 graduated mfxer / mtxer into the decoder; the
        // SPR-level directories drop their SPR-1 rows.
        assert!(spr_gap(SprDirection::MfSpr, 1).is_none());
        assert!(spr_gap(SprDirection::MtSpr, 1).is_none());
    }

    #[test]
    fn primary31_x_form_residue_no_longer_in_directory() {
        // Stage 40C.9 graduated tw / td / popcntb / mcrxr into the
        // decoder. With that landing, the primary-31 X/XO directory is
        // empty; every encoding now decodes to a known variant.
        for xo in [4u16, 68, 122, 512] {
            assert!(
                opcode_gap(31, xo).is_none(),
                "primary-31 X-form residue XO {xo} still present in OPCODE_GAPS"
            );
        }
        assert!(
            OPCODE_GAPS.is_empty(),
            "OPCODE_GAPS expected empty after Stage 40C.9; entries: {OPCODE_GAPS:?}"
        );
    }

    #[test]
    fn byte_reverse_family_no_longer_in_directory() {
        // Stage 40C.3 graduated ldbrx / lwbrx / lhbrx / sdbrx /
        // stwbrx / sthbrx into the decoder. The sdbrx vs stdbrx
        // mnemonic-mapping note still applies: the variant in
        // PpuInstruction is canonically `Sdbrx`, matching the
        // CBE-Handbook A.2.1 definition page; the Stage 40D
        // harness handles the upstream `stdbrx` spelling.
        for xo in [532u16, 534, 660, 662, 790, 918] {
            assert!(
                opcode_gap(31, xo).is_none(),
                "byte-reverse XO {xo} still present in OPCODE_GAPS"
            );
        }
    }

    #[test]
    fn unknown_opcode_returns_none() {
        // XO 5 in primary 31 has no documented encoding.
        assert!(opcode_gap(31, 5).is_none());
        // Primary 0 has no entries at all.
        assert!(opcode_gap(0, 0).is_none());
    }

    #[test]
    fn spr_18_resolves_differently_by_direction() {
        // The MfSpr / MtSpr directories share most SPR rows; the
        // direction-keyed lookup must still pick the right mnemonic.
        // SPR 1 (xer) is no longer a gap after Stage 40C.10; SPR 18
        // (dsisr) is the new exemplar.
        let read = spr_gap(SprDirection::MfSpr, 18).expect("mfdsisr must be present");
        assert_eq!(read.mnemonic, "mfdsisr");
        let write = spr_gap(SprDirection::MtSpr, 18).expect("mtdsisr must be present");
        assert_eq!(write.mnemonic, "mtdsisr");
    }

    #[test]
    fn mftb_sub_table_is_empty() {
        assert!(spr_gap(SprDirection::MfTb, 268).is_none());
        assert!(spr_gap(SprDirection::MfTb, 269).is_none());
        assert!(spr_gap(SprDirection::MfTb, 999).is_none());
    }

    #[test]
    fn unknown_spr_returns_none() {
        // SPR 99 isn't in any sub-table.
        assert!(spr_gap(SprDirection::MfSpr, 99).is_none());
        assert!(spr_gap(SprDirection::MtSpr, 99).is_none());
    }

    #[test]
    fn spr_direction_op_mnemonic_matches_xfx_opcodes() {
        assert_eq!(SprDirection::MfSpr.op_mnemonic(), "mfspr");
        assert_eq!(SprDirection::MfTb.op_mnemonic(), "mftb");
        assert_eq!(SprDirection::MtSpr.op_mnemonic(), "mtspr");
    }
}
