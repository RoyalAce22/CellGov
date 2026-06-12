//! Decoder fall-through: rejection modes, gap-table disjointness, and XO-key collisions.

use super::*;

#[test]
fn unknown_primary_reports_encoding_not_recognized() {
    // Primary 2 (`tdi`, trap-dword-immediate) has no top-level
    // arm in CellGov and no opcode-gap row, so the top-level
    // fall-through surfaces it as EncodingNotRecognized with
    // only the raw word in the Display.
    let raw = 0x0800_0000;
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        other => panic!("expected EncodingNotRecognized, got {other:?}"),
    }
}

#[test]
fn mfspr_with_unsupported_spr_reports_named_spr_locator() {
    // mfspr rT, SPR=18 reads DSISR (supervisor). The XFX SPR field
    // is split: LSB-0 bits 16..20 (the decoder's `ra` slot) hold
    // SPR LOW 5 bits; LSB-0 bits 11..15 (`rb` slot) hold SPR HIGH
    // 5 bits; the decoder reassembles `spr = (rb << 5) | ra`. For
    // SPR=18: low=18, high=0. (SPR 1 graduated to `mfxer` in
    // Stage 40C.10, so the smoke test moved to the next named gap.)
    let raw: u32 = (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (0u32 << 11) | (339u32 << 1);
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Spr { op_mnemonic, spr },
            mnemonic,
            ..
        } => {
            assert_eq!(op_mnemonic, "mfspr");
            assert_eq!(spr, 18);
            assert_eq!(mnemonic, "mfdsisr");
        }
        other => panic!("expected DecoderArmUnimplemented (Spr), got {other:?}"),
    }
    assert_eq!(err.to_string(), "missing mfdsisr (mfspr, SPR 18)");
}

/// Synthesize a raw 32-bit word that should hit `(primary, xo)`
/// in the appropriate decoder path, respecting the form's bit
/// placement. The bottom of this file holds the per-form
/// encoders the disjointness test uses; if a future row hits a
/// form not in the match, the synthesizer panics rather than
/// produce a wrong word (which would manufacture a false
/// disjointness-test failure).
fn synth_opcode(primary: u8, xo: u16) -> u32 {
    let p = primary as u32;
    let x = xo as u32;
    match primary {
        // MD-form (3-bit sub-XO at LSB-0 bits 2..4); MDS-form
        // (4-bit sub-XO at LSB-0 bits 1..4 -- bit 4 of the
        // 4-bit field carries over from the bit-30 SH-hi slot
        // of MD-form). For primary 30 the directory holds
        // both MD and MDS rows: route MDS values (xo > 3) via
        // the 4-bit encoding, MD values via the 3-bit one.
        30 => {
            if x > 3 {
                (p << 26) | (x << 1)
            } else {
                (p << 26) | (x << 2)
            }
        }
        // DS-form (low 2 bits select the sub-op). Primaries
        // 58 / 62 are the directory's only DS-form entries.
        58 | 62 => (p << 26) | (x & 0x3),
        // X-form / XO-form for primary 31: XO at LSB-0 bits
        // 1..10 with the bit-0 (Rc) set to 0. The decoder
        // tries 9-bit XO first (XO-form) then 10-bit (X-form);
        // synthesizing at the 10-bit position covers both
        // because the 9-bit lookup masks to 0x1FF and the
        // 10-bit to 0x3FF, so a clean 10-bit XO either hits
        // both interpretations or only the X-form one.
        31 => (p << 26) | (x << 1),
        // XL-form for primary 19: XO at LSB-0 bits 1..10
        // (same shape as X-form's 10-bit XO).
        19 => (p << 26) | (x << 1),
        // D / I / B-form primaries that have no XO field
        // (the directory keys these with xo=0). The
        // synthesizer just sets the primary.
        43 | 46 | 47 | 49 | 51 => p << 26,
        // Anything else means the directory grew a row whose
        // form isn't yet handled by the synthesizer. Refuse
        // to manufacture a wrong word.
        _ => panic!(
            "synth_opcode: primary {primary} (xo {xo}) needs a form encoder; \
             add it to the match in decode.rs::tests::synth_opcode"
        ),
    }
}

/// Synthesize a raw word for an SPR / TBR-keyed row: place the
/// SPR's low 5 bits in `ra` (LSB-0 16..20) and the high 5 bits
/// in `rb` (LSB-0 11..15) per the XFX-form half-swap.
fn synth_spr(spr: u16, xo: u32) -> u32 {
    let low5 = (spr & 0x1F) as u32;
    let high5 = ((spr >> 5) & 0x1F) as u32;
    (31u32 << 26) | (0u32 << 21) | (low5 << 16) | (high5 << 11) | (xo << 1)
}

#[test]
fn opcode_gaps_are_disjoint_from_decoder_arms() {
    for row in known_encodings::OPCODE_GAPS {
        let raw = synth_opcode(row.primary, row.xo);
        match decode(raw) {
            Ok(inst) => panic!(
                "OPCODE_GAPS row primary {p}, xo {x}, mnemonic {m}: \
                 decode returned Ok({inst:?}) -- the decoder grew an \
                 arm; delete this row from OPCODE_GAPS",
                p = row.primary,
                x = row.xo,
                m = row.mnemonic
            ),
            Err(PpuDecodeError::DecoderArmUnimplemented {
                locator: Locator::Opcode { primary, xo },
                mnemonic,
                ..
            }) => {
                assert_eq!(
                    (primary, xo, mnemonic),
                    (row.primary, row.xo, row.mnemonic),
                    "OPCODE_GAPS row {row:?}: synth word decoded to a \
                     DIFFERENT row's locator -- two rows are colliding"
                );
            }
            Err(PpuDecodeError::DecoderArmUnimplemented {
                locator, mnemonic, ..
            }) => panic!(
                "OPCODE_GAPS row {row:?}: locator {locator:?} mnemonic \
                 {mnemonic} -- expected Opcode locator"
            ),
            Err(PpuDecodeError::EncodingNotRecognized { raw: r }) => panic!(
                "OPCODE_GAPS row {row:?}: synth word 0x{r:08x} surfaced as \
                 EncodingNotRecognized -- the synthesizer is producing a \
                 word that misses the directory lookup, or the row's \
                 mnemonic is stale"
            ),
        }
    }
}

#[test]
fn spr_gaps_are_disjoint_from_decoder_arms() {
    let cases: &[(SprDirection, &[known_encodings::SprGap], u32)] = &[
        (SprDirection::MfSpr, known_encodings::MFSPR_GAPS, 339),
        (SprDirection::MfTb, known_encodings::MFTB_GAPS, 371),
        (SprDirection::MtSpr, known_encodings::MTSPR_GAPS, 467),
    ];
    for (direction, table, xo) in cases {
        for row in *table {
            let raw = synth_spr(row.spr, *xo);
            match decode(raw) {
                Ok(inst) => panic!(
                    "SPR-gap row direction {direction:?}, spr {spr}, mnemonic \
                     {m}: decode returned Ok({inst:?}) -- the SPR/TBR arm \
                     now handles this selector; delete this row",
                    spr = row.spr,
                    m = row.mnemonic
                ),
                Err(PpuDecodeError::DecoderArmUnimplemented {
                    locator: Locator::Spr { op_mnemonic, spr },
                    mnemonic,
                    ..
                }) => {
                    assert_eq!(op_mnemonic, direction.op_mnemonic());
                    assert_eq!(spr, row.spr);
                    assert_eq!(mnemonic, row.mnemonic);
                }
                Err(other) => panic!(
                    "SPR-gap row direction {direction:?}, spr {spr}, mnemonic \
                     {m}: got {other:?} -- expected DecoderArmUnimplemented \
                     (Spr)",
                    spr = row.spr,
                    m = row.mnemonic
                ),
            }
        }
    }
}

#[test]
fn primary_zero_always_rejects_as_encoding_not_recognized() {
    // The prescan bucket's safety premise: every primary-0
    // word rejects as EncodingNotRecognized (never decodes to
    // a real instruction, never matches a DecoderArmUnimplemented
    // gap). If a future arm started accepting any primary-0
    // encoding, the bucket would silently launder a real
    // instruction into the data-in-text line.
    for bits in [
        0x0000_0000u32,
        0x0000_0001,
        0x03FF_FFFF,
        0x0123_4567,
        0x02AA_AAAA,
    ] {
        match decode(bits) {
            Err(PpuDecodeError::EncodingNotRecognized { raw }) => assert_eq!(raw, bits),
            other => {
                panic!("primary-0 word {bits:#010x} must be EncodingNotRecognized, got {other:?}")
            }
        }
    }
}

#[test]
fn primary4_unknown_vx_xo_rejects_does_not_fabricate_stub() {
    // The decoder previously routed any primary-4 word with
    // xo_6 NOT in 0x20..=0x2F through `_ => Ok(Vx { xo })`,
    // silently fabricating a typed stub for AltiVec-PEM
    // unmapped encodings. After S1 the catch-all is bounded
    // by KNOWN_VX_XOS. Pick xo_11=1 (odd low value, no PEM
    // assignment): must reject.
    let raw: u32 = (4u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | 1;
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        other => panic!("expected EncodingNotRecognized, got {other:?}"),
    }
    // Sanity: a known VX (vmaxub at XO 2) still decodes as Vx.
    let raw_known: u32 = (4u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | 2;
    match decode(raw_known).unwrap() {
        PpuInstruction::Vx {
            xo: 2,
            vt: 3,
            va: 4,
            vb: 5,
        } => {}
        other => panic!("expected Vx xo=2, got {other:?}"),
    }
}

#[test]
fn primary4_unknown_va_xo_rejects_does_not_fabricate_stub() {
    // xo_6 = 35 sits in the VA-form range 0x20..=0x2F but is
    // not assigned by AltiVec-PEM (no instruction defined).
    // Pre-S1 this fabricated `Va { xo: 35 }`.
    let raw: u32 = (4u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (6u32 << 6) | 35;
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        other => panic!("expected EncodingNotRecognized, got {other:?}"),
    }
    // Sanity: vsel (XO 42) still decodes as a Va stub.
    let raw_known: u32 =
        (4u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (6u32 << 6) | 42;
    match decode(raw_known).unwrap() {
        PpuInstruction::Va {
            xo: 42,
            vt: 3,
            va: 4,
            vb: 5,
            vc: 6,
        } => {}
        other => panic!("expected Va xo=42, got {other:?}"),
    }
}

#[test]
fn srawi_sradi_xo10_keys_do_not_collide_with_xo9_first_pass() {
    // decode_x31 runs an `xo_9 = (raw >> 1) & 0x1FF` match first;
    // on miss it falls through to the 10-bit XO match where
    // srawi (824), sradi (826/827) live. The fall-through only
    // works because 824, 826, 827 mask to xo_9 projections
    // (312, 314, 315) that are NOT in the xo_9 first-pass arms.
    // This test pins that non-collision: if a future arm adds
    // xo_9 = 312/314/315, srawi/sradi will silently decode as
    // the wrong instruction.
    //
    // Build minimal encodings: srawi (824) with sh=0 in rb slot;
    // sradi (826, sh_hi=0) and (827, sh_hi=1) with sh_lo=0.
    let p31 = 31u32 << 26;
    let srawi_raw = p31 | (3u32 << 21) | (4u32 << 16) | (824u32 << 1);
    let sradi_lo_raw = p31 | (3u32 << 21) | (4u32 << 16) | (826u32 << 1);
    let sradi_hi_raw = p31 | (3u32 << 21) | (4u32 << 16) | (827u32 << 1);
    match decode(srawi_raw).unwrap() {
        PpuInstruction::Srawi { .. } => {}
        other => panic!("srawi xo_9=312 collision: got {other:?}"),
    }
    match decode(sradi_lo_raw).unwrap() {
        PpuInstruction::Sradi { .. } => {}
        other => panic!("sradi xo_9=314 collision: got {other:?}"),
    }
    match decode(sradi_hi_raw).unwrap() {
        PpuInstruction::Sradi { .. } => {}
        other => panic!("sradi xo_9=315 collision: got {other:?}"),
    }
}
