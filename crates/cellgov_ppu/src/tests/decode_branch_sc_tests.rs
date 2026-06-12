//! Branch, sc, and Rc-mandatory conditional-store decode gates.

use super::*;

#[test]
fn sc_decodes() {
    // sc -> primary opcode 17 -> 0x44000002, LEV=0.
    let raw = 0x4400_0002;
    let insn = decode(raw).unwrap();
    assert_eq!(insn, PpuInstruction::Sc { lev: 0 });
}

#[test]
fn sc_preserves_lev_field() {
    // LEV=1 is the LV1 hypercall form; LEV occupies raw bits
    // 5..=11 (PPC bits 20..=26). Build LEV=1 and LEV=5.
    for lev in [1u8, 5, 0x7F] {
        let raw: u32 = (17u32 << 26) | ((lev as u32) << 5) | 2;
        let insn = decode(raw).unwrap();
        assert_eq!(insn, PpuInstruction::Sc { lev });
    }
}

#[test]
fn blr_decodes() {
    // blr -> bclr 20,0,0 -> 0x4E800020
    let raw = 0x4E80_0020;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Bclr {
            bo: 20,
            bi: 0,
            link: false
        }
    );
}

#[test]
fn bl_decodes() {
    // bl +8 -> 0x48000009
    let raw = 0x4800_0009;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::B {
            offset: 8,
            aa: false,
            link: true
        }
    );
}

#[test]
fn sc_with_bit30_clear_rejects() {
    // [PPC-Book1 p:8 s:1.7.3 SC-Form] bit 30 must be 1. A
    // primary-17 word with bit-30 clear is an illegal
    // / reserved form, not `sc`. Must reject rather than
    // route to syscall dispatch with junk lev.
    let raw: u32 = 17u32 << 26; // bit 30 = 0
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        PpuDecodeError::DecoderArmUnimplemented { raw: r, .. } => assert_eq!(r, raw),
    }
}

#[test]
fn stwcx_with_rc_clear_rejects() {
    // [PPC-Book2 p:25 s:3.3] stwcx. is always Rc-set in the mnemonic.
    // An encoding with XO=150 and Rc=0 is a reserved form; the
    // decoder must reject rather than silently producing Stwcx.
    // Build: primary 31, RS=1, RA=2, RB=3, XO=150 (10-bit at
    // bits 21..31), Rc=0.
    let raw: u32 = (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (150u32 << 1);
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        PpuDecodeError::DecoderArmUnimplemented { raw: r, .. } => assert_eq!(r, raw),
    }
}

#[test]
fn stwcx_with_rc_set_decodes() {
    let raw: u32 = (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (150u32 << 1) | 1;
    match decode(raw).unwrap() {
        PpuInstruction::Stwcx {
            rs: 1,
            ra: 2,
            rb: 3,
        } => {}
        other => panic!("expected Stwcx rs=1 ra=2 rb=3, got {other:?}"),
    }
}

#[test]
fn stdcx_with_rc_clear_rejects() {
    // [PPC-Book2 p:25 s:3.3] stdcx. always Rc-set; XO=214 with Rc=0
    // is reserved and must reject.
    let raw: u32 = (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (214u32 << 1);
    let err = decode(raw).unwrap_err();
    match err {
        PpuDecodeError::EncodingNotRecognized { raw: r } => assert_eq!(r, raw),
        PpuDecodeError::DecoderArmUnimplemented { raw: r, .. } => assert_eq!(r, raw),
    }
}

#[test]
fn stdcx_with_rc_set_decodes() {
    let raw: u32 = (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (214u32 << 1) | 1;
    match decode(raw).unwrap() {
        PpuInstruction::Stdcx {
            rs: 1,
            ra: 2,
            rb: 3,
        } => {}
        other => panic!("expected Stdcx rs=1 ra=2 rb=3, got {other:?}"),
    }
}

#[test]
fn ba_decodes_absolute_unconditional_branch() {
    // ba target_addr: primary 18, AA=1, LK=0. Encoding:
    // (18 << 26) | (li & 0x03FFFFFC) | (AA << 1) | LK.
    // Use li = 0x100 (4 << 6 -> sign-positive, byte target 0x100).
    let raw: u32 = (18u32 << 26) | 0x100 | 0b10;
    let insn = decode(raw).unwrap();
    match insn {
        PpuInstruction::B { offset, aa, link } => {
            assert_eq!(offset, 0x100);
            assert!(aa, "AA bit must be set for `ba`");
            assert!(!link, "LK bit must be clear for non-link branch");
        }
        other => panic!("expected B, got {other:?}"),
    }
}

#[test]
fn bla_decodes_absolute_link_branch() {
    // bla target_addr: primary 18, AA=1, LK=1.
    let raw: u32 = (18u32 << 26) | 0x200 | 0b11;
    match decode(raw).unwrap() {
        PpuInstruction::B {
            offset: 0x200,
            aa: true,
            link: true,
        } => {}
        other => panic!("expected B aa+link, got {other:?}"),
    }
}

#[test]
fn bca_decodes_absolute_conditional_branch() {
    // bca bo, bi, target: primary 16, AA=1, LK=0.
    // Encoding: (16<<26) | (BO<<21) | (BI<<16) | (BD<<2) | (AA<<1) | LK.
    // BO=12 (branch if true), BI=2, BD=0x10.
    let raw: u32 = (16u32 << 26) | (12u32 << 21) | (2u32 << 16) | (0x10u32 << 2) | 0b10;
    match decode(raw).unwrap() {
        PpuInstruction::Bc {
            bo: 12,
            bi: 2,
            offset,
            aa: true,
            link: false,
        } => {
            assert_eq!(offset, 0x40);
        }
        other => panic!("expected Bc with aa=true, got {other:?}"),
    }
}
