//! PpuDecodeError Display formatting across locator variants.

use super::*;

#[test]
fn opcode_locator_display_names_primary_and_xo() {
    let err = PpuDecodeError::DecoderArmUnimplemented {
        locator: Locator::Opcode {
            primary: 31,
            xo: 38,
        },
        mnemonic: "lvsr",
        raw: 0x7d80_484c,
    };
    assert_eq!(err.to_string(), "missing lvsr (primary 31, xo 38)");
}

#[test]
fn spr_locator_display_names_op_and_spr() {
    let err = PpuDecodeError::DecoderArmUnimplemented {
        locator: Locator::Spr {
            op_mnemonic: "mfspr",
            spr: 1,
        },
        mnemonic: "mfxer",
        raw: 0,
    };
    assert_eq!(err.to_string(), "missing mfxer (mfspr, SPR 1)");
}

#[test]
fn unrecognized_display_carries_only_raw() {
    let err = PpuDecodeError::EncodingNotRecognized { raw: 0xdead_beef };
    assert_eq!(err.to_string(), "no documented encoding for raw 0xdeadbeef");
}

#[test]
fn no_p_tbd_or_doc_key_in_display() {
    let cases = [
        PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Opcode {
                primary: 31,
                xo: 38,
            },
            mnemonic: "lvsr",
            raw: 0x7d80_484c,
        },
        PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Spr {
                op_mnemonic: "mfspr",
                spr: 1,
            },
            mnemonic: "mfxer",
            raw: 0,
        },
        PpuDecodeError::EncodingNotRecognized { raw: 0x0800_0000 },
    ];
    for err in cases {
        let text = err.to_string();
        assert!(!text.contains("p:TBD"), "p:TBD in {text:?}");
        assert!(!text.contains("DOC-KEY"), "DOC-KEY in {text:?}");
        assert!(!text.contains("AltiVec-PEM"), "spec-doc tag in {text:?}");
        assert!(!text.contains("PPC-Book"), "spec-doc tag in {text:?}");
        assert!(!text.contains("CBE-Handbook"), "spec-doc tag in {text:?}");
    }
}

#[test]
fn raw_accessor_returns_word_for_both_variants() {
    let arm = PpuDecodeError::DecoderArmUnimplemented {
        locator: Locator::Opcode {
            primary: 31,
            xo: 38,
        },
        mnemonic: "lvsr",
        raw: 0xCAFE_BABE,
    };
    assert_eq!(arm.raw(), 0xCAFE_BABE);
    let unk = PpuDecodeError::EncodingNotRecognized { raw: 0xFEED_FACE };
    assert_eq!(unk.raw(), 0xFEED_FACE);
}
