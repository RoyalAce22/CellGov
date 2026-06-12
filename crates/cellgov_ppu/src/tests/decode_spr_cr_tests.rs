//! SPR selector decoding (XER / VRSAVE / TB) and CR move / logical forms.

use super::*;

#[test]
fn mfspr_at_spr_1_decodes_to_mfxer() {
    // SPR=1 reads XER. Canonical encoding: ra=1, rb=0 (low half
    // ahead of high half per the XFX split).
    let raw: u32 = (31u32 << 26) | (5u32 << 21) | (1u32 << 16) | (339u32 << 1);
    assert_eq!(decode(raw).unwrap(), PpuInstruction::Mfxer { rt: 5 });
}

#[test]
fn mtspr_at_supervisor_spr_names_correct_mtspr_mnemonic() {
    // Same encoding pattern as the mfspr test above, but with
    // XO 467 (mtspr direction). SPR=18 routes to mtdsisr after
    // Stage 40C.10 graduated SPR 1 (mtxer) into the decoder.
    let raw: u32 = (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (0u32 << 11) | (467u32 << 1);
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Spr { op_mnemonic, spr },
            mnemonic,
            ..
        } => {
            assert_eq!(op_mnemonic, "mtspr");
            assert_eq!(spr, 18);
            assert_eq!(mnemonic, "mtdsisr");
        }
        other => panic!("expected DecoderArmUnimplemented (Spr), got {other:?}"),
    }
    assert_eq!(err.to_string(), "missing mtdsisr (mtspr, SPR 18)");
}

#[test]
fn mtspr_at_spr_1_decodes_to_mtxer() {
    // SPR=1 writes XER.
    let raw: u32 = (31u32 << 26) | (7u32 << 21) | (1u32 << 16) | (467u32 << 1);
    assert_eq!(decode(raw).unwrap(), PpuInstruction::Mtxer { rs: 7 });
}

#[test]
fn mfspr_at_spr_256_decodes_to_mfvrsave() {
    // SPR=256: half-swap encodes as rb=8 (high5), ra=0 (low5).
    // 0x7c0042a6 is the actual SSHD/WipEout production word for
    // `mfvrsave r0` -- pinning the real-world site, not a
    // synthetic encoding.
    let raw: u32 = 0x7c00_42a6;
    assert_eq!(decode(raw).unwrap(), PpuInstruction::Mfvrsave { rt: 0 });
}

#[test]
fn mtspr_at_spr_256_decodes_to_mtvrsave() {
    // 0x7c0043a6 is the SSHD/WipEout production word for
    // `mtvrsave r0` -- the paired write to VRSAVE.
    let raw: u32 = 0x7c00_43a6;
    assert_eq!(decode(raw).unwrap(), PpuInstruction::Mtvrsave { rs: 0 });
}

#[test]
fn crnor_decodes_self_alias_form() {
    // PowerPC `crnot Bx, By` mnemonic decomposes into
    // `crnor Bx, By, By`; this tests the self-alias case
    // (BA == BB), with the encoding for crnor cr30, cr29, cr29.
    // Encoding: OP=19 | BT=30 | BA=29 | BB=29 | XO=33 | 0
    let raw = 0x4FDD_E842;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Crnor {
            bt: 30,
            ba: 29,
            bb: 29
        }
    );
}

#[test]
fn mcrf_decodes() {
    // mcrf 5, 2: BF=5 at bits 6..9, BFA=2 at bits 11..14, XO=0.
    let raw = (19u32 << 26) | (5u32 << 23) | (2u32 << 18);
    let insn = decode(raw).unwrap();
    assert_eq!(insn, PpuInstruction::Mcrf { crfd: 5, crfs: 2 });
}

#[test]
fn mftb_decodes_lower_tbr() {
    // mftb r3 -> primary 31, XO=371, TBR=268.
    // TBR field uses the SPR swap: spr_raw = (rb<<5)|ra, so TBR=268
    // encodes as rb = 268>>5 = 8, ra = 268 & 0x1F = 12.
    let raw: u32 = (31u32 << 26) | (3u32 << 21) | (12u32 << 16) | (8u32 << 11) | (371u32 << 1);
    let insn = decode(raw).unwrap();
    assert_eq!(insn, PpuInstruction::Mftb { rt: 3 });
}

#[test]
fn mftbu_decodes_upper_tbr() {
    // mftbu r3 -> TBR=269 -> rb = 269>>5 = 8, ra = 269 & 0x1F = 13.
    let raw: u32 = (31u32 << 26) | (3u32 << 21) | (13u32 << 16) | (8u32 << 11) | (371u32 << 1);
    let insn = decode(raw).unwrap();
    assert_eq!(insn, PpuInstruction::Mftbu { rt: 3 });
}

#[test]
fn mfspr_268_and_269_decode_as_mftb_and_mftbu() {
    // [PPC-Book2 p:30 s:4.2] mfspr RT, 268/269 is an alternate
    // spelling of mftb / mftbu. The XFX half-swap encodes SPR=268
    // as ra=12 (low 5 bits), rb=8 (high 5 bits), which assembles
    // to (rb << 5) | ra = 0x10C = 268. SPR=269 -> ra=13, rb=8.
    let raw_268: u32 = (31u32 << 26)
        | (3u32 << 21)
        | (12u32 << 16) // SPR low half
        | (8u32 << 11)  // SPR high half
        | (339u32 << 1);
    assert_eq!(decode(raw_268).unwrap(), PpuInstruction::Mftb { rt: 3 });

    let raw_269: u32 = (31u32 << 26) | (3u32 << 21) | (13u32 << 16) | (8u32 << 11) | (339u32 << 1);
    assert_eq!(decode(raw_269).unwrap(), PpuInstruction::Mftbu { rt: 3 });
}

#[test]
fn mfocrf_form_decodes_to_typed_variant_not_mfcr() {
    // mfocrf shares XO 19 with mfcr, distinguished by PPC bit 11
    // (raw bit 20) being set. mfocrf reads one CR field; mfcr
    // reads all eight. The bit-11 marker routes to a typed
    // Mfocrf variant so the executor applies the one-field
    // semantic rather than masquerading as mfcr.
    let raw: u32 = (31u32 << 26)
        | (3u32 << 21)
        | (1u32 << 20) // bit 20 set => mfocrf
        | (0x80u32 << 12)
        | (19u32 << 1);
    match decode(raw).unwrap() {
        PpuInstruction::Mfocrf { rt: 3, crm: 0x80 } => {}
        other => panic!("expected Mfocrf, got {other:?}"),
    }
    // Sanity: with bit 20 clear, the same encoding decodes to Mfcr.
    let raw_mfcr: u32 = (31u32 << 26) | (3u32 << 21) | (19u32 << 1);
    match decode(raw_mfcr).unwrap() {
        PpuInstruction::Mfcr { rt: 3 } => {}
        other => panic!("expected Mfcr, got {other:?}"),
    }
}

#[test]
fn mtocrf_form_decodes_to_typed_variant_not_mtcrf() {
    // mtocrf shares XO 144 with mtcrf, distinguished by PPC bit 11
    // (raw bit 20) being set. Semantics differ -- one-hot CRM
    // selects a single field. Routes to a typed Mtocrf variant
    // so the executor handles the one-field semantic distinctly.
    let raw: u32 = (31u32 << 26)
        | (3u32 << 21)
        | (1u32 << 20) // bit 20 set => mtocrf
        | (0x80u32 << 12)
        | (144u32 << 1);
    match decode(raw).unwrap() {
        PpuInstruction::Mtocrf { rs: 3, crm: 0x80 } => {}
        other => panic!("expected Mtocrf, got {other:?}"),
    }
    // Sanity: with bit 20 clear, the same encoding decodes to Mtcrf.
    let raw_mtcrf: u32 = (31u32 << 26) | (3u32 << 21) | (0x80u32 << 12) | (144u32 << 1);
    match decode(raw_mtcrf).unwrap() {
        PpuInstruction::Mtcrf { rs: 3, crm: 0x80 } => {}
        other => panic!("expected Mtcrf, got {other:?}"),
    }
}
