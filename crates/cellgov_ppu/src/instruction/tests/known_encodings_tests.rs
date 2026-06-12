//! Known-encoding gap tables stay sorted for binary search.

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
