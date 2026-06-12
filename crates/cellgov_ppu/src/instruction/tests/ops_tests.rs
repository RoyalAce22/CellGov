//! Encoding-law tests: re-derive the AltiVec opcode-map geometry
//! from the mnemonic strings, independently of the enum
//! discriminants, so a transcription slip must produce two
//! correlated errors to survive.

use std::collections::BTreeMap;

use strum::IntoEnumIterator;

use super::{Fp59Op, Fp63Op, VaOp, VxOp};

fn vx_names() -> BTreeMap<&'static str, u16> {
    VxOp::iter()
        .map(|op| (<&'static str>::from(op), op as u16))
        .collect()
}

#[test]
fn census_law() {
    // 115 VX ops (vxor included) + 13 VXR compare bases.
    assert_eq!(VxOp::iter().count(), 128);
    assert_eq!(VxOp::iter().filter(|op| op.is_vxr_compare()).count(), 13);
    assert_eq!(VaOp::iter().count(), 14);
    assert_eq!(Fp63Op::iter().count(), 29);
    assert_eq!(Fp59Op::iter().count(), 10);
    // Mnemonics are unique (a duplicate name would alias two XOs).
    assert_eq!(vx_names().len(), 128);
}

/// For every mnemonic pair differing only in the `b`/`h` (`w`)
/// element-width letter, the XOs step by exactly +64 (+128).
#[test]
fn width_stride_law() {
    let names = vx_names();
    let mut checked = 0usize;
    for (&name, &xo) in &names {
        // The sum-across group is irregular: there is no vsum4uhs
        // slot, so vsum4sbs -> vsum4shs is not a +64 step (1800 vs
        // 1608). Signedness still holds for it and is covered by
        // signedness_law.
        if name.starts_with("vsum4") {
            continue;
        }
        for (pos, ch) in name.char_indices() {
            if ch != 'b' {
                continue;
            }
            let mut h_name = name.to_string();
            h_name.replace_range(pos..pos + 1, "h");
            let mut w_name = name.to_string();
            w_name.replace_range(pos..pos + 1, "w");
            if let Some(&h_xo) = names.get(h_name.as_str()) {
                assert_eq!(h_xo, xo + 64, "{name} -> {h_name} stride violated");
                checked += 1;
            }
            if let Some(&w_xo) = names.get(w_name.as_str()) {
                assert_eq!(w_xo, xo + 128, "{name} -> {w_name} stride violated");
                checked += 1;
            }
        }
    }
    assert!(checked >= 40, "width law only checked {checked} pairs");
}

/// For every signed/unsigned pair in the compare, min/max, average,
/// multiply-odd/even, sum-across, and saturating add/sub families,
/// signed XO == unsigned XO + 256. This is the precise law the
/// audited third-party tables violate.
#[test]
fn signedness_law() {
    let names = vx_names();
    let mut checked = 0usize;
    const U_FAMILIES: &[&str] = &[
        "vmaxu", "vminu", "vavgu", "vcmpgtu", "vmulou", "vmuleu", "vsum4u",
    ];
    for (&name, &xo) in &names {
        for fam in U_FAMILIES {
            if let Some(rest) = name.strip_prefix(fam) {
                let s_name = format!("{}s{rest}", &fam[..fam.len() - 1]);
                if let Some(&s_xo) = names.get(s_name.as_str()) {
                    assert_eq!(s_xo, xo + 256, "{name} -> {s_name} signedness violated");
                    checked += 1;
                }
            }
        }
        // Saturating add/sub: vaddubs -> vaddsbs etc.
        if (name.starts_with("vaddu") || name.starts_with("vsubu")) && name.ends_with('s') {
            let s_name = name.replacen("addu", "adds", 1).replacen("subu", "subs", 1);
            if let Some(&s_xo) = names.get(s_name.as_str()) {
                assert_eq!(s_xo, xo + 256, "{name} -> {s_name} signedness violated");
                checked += 1;
            }
        }
    }
    assert!(checked >= 20, "signedness law only checked {checked} pairs");
}

/// For add/sub families, the saturating XO == the modulo XO + 512.
#[test]
fn saturate_law() {
    let names = vx_names();
    let mut checked = 0usize;
    for (&name, &xo) in &names {
        if !(name.starts_with("vadd") || name.starts_with("vsub")) || !name.ends_with('m') {
            continue;
        }
        let s_name = format!("{}s", &name[..name.len() - 1]);
        if let Some(&s_xo) = names.get(s_name.as_str()) {
            assert_eq!(s_xo, xo + 512, "{name} -> {s_name} saturate violated");
            checked += 1;
        }
    }
    assert_eq!(checked, 6, "expected exactly the 6 add/sub modulo ops");
}

/// Rc=1 compare forms are not separate variants: `1024 | base`
/// resolves to the base op with `rc = true`, and only for the 13
/// VXR compare bases.
#[test]
fn vxr_rc_pairing_law() {
    let mut vxr = 0usize;
    for op in VxOp::iter() {
        let base = op as u16;
        if op.is_vxr_compare() {
            vxr += 1;
            assert!(base < 1024, "VXR base {base} must fit 10 bits");
            assert_eq!(
                VxOp::from_repr(1024 | base),
                None,
                "Rc form of {op:?} must not be its own variant"
            );
            assert_eq!(VxOp::decode(1024 | base), Some((op, true)));
        }
        // Every discriminant decodes to itself with rc = false --
        // including non-VXR ops whose discriminant has bit 10 set
        // (vand 1028, vor 1156, ...), which must NOT read as Rc
        // forms.
        assert_eq!(VxOp::decode(base), Some((op, false)));
    }
    assert_eq!(vxr, 13);
    // An Rc-bit pattern over a non-compare op rejects outright
    // unless it collides with a real discriminant (in which case
    // the direct hit already won above): vrlb (4) has no Rc form
    // and 1024|4 = 1028 is vand, not "vrlb.".
    assert_eq!(VxOp::decode(1028), Some((VxOp::Vand, false)));
    // A genuinely unassigned XO rejects.
    assert_eq!(VxOp::decode(2046), None);
    assert_eq!(VxOp::decode(1), None);
}

/// Every op round-trips through `from_xo`; A-form ops additionally
/// resolve with any FRC value riding in the upper XO bits. An
/// X-form op whose low 5 bits aliased an A-form discriminant would
/// fail its own round-trip, so the split is covered by the first
/// assertion.
#[test]
fn fp63_form_split_law() {
    for op in Fp63Op::iter() {
        let xo = op as u16;
        assert_eq!(Fp63Op::from_xo(xo), Some(op), "{op:?} failed round-trip");
        if op.is_a_form() {
            assert_eq!(Fp63Op::from_xo((31 << 5) | xo), Some(op));
        }
    }
    assert_eq!(Fp63Op::iter().filter(|op| op.is_a_form()).count(), 11);
    assert_eq!(Fp63Op::from_xo(999), None);
}

#[test]
fn fp59_round_trip() {
    for op in Fp59Op::iter() {
        assert_eq!(Fp59Op::from_xo(op as u16), Some(op));
        assert_eq!(Fp59Op::from_xo((17 << 5) | op as u16), Some(op));
    }
    assert_eq!(Fp59Op::from_xo(23), None);
}

/// Reserved encoding slots stay rejected: VA XOs 35 and 45 are
/// unassigned within 0x20..=0x2F, and odd VX positions have no
/// AltiVec-PEM instruction.
#[test]
fn reserved_slots_reject() {
    assert_eq!(VaOp::from_repr(35), None);
    assert_eq!(VaOp::from_repr(45), None);
    for xo in [1u16, 3, 5, 7, 9, 11, 13, 15] {
        assert_eq!(VxOp::decode(xo), None, "odd VX slot {xo} must reject");
    }
}

/// The mnemonic is the lowercase variant name -- spot anchors so a
/// rename can't silently change rendered text.
#[test]
fn mnemonic_serialization_anchors() {
    assert_eq!(<&'static str>::from(VxOp::Vaddubm), "vaddubm");
    assert_eq!(<&'static str>::from(VxOp::Vcmpgtsw), "vcmpgtsw");
    assert_eq!(<&'static str>::from(VaOp::Vmhraddshs), "vmhraddshs");
    assert_eq!(<&'static str>::from(Fp63Op::Fctidz), "fctidz");
    assert_eq!(<&'static str>::from(Fp59Op::Fnmadds), "fnmadds");
}

/// Dev-only differential spot-check: every op-enum mnemonic must
/// exist as a `PPUDisAsm` method in the RPCS3 source tree. RPCS3 is
/// evidence, not authority -- a miss here gets triaged against the
/// ISA PDFs by hand. Skips cleanly when `tools/rpcs3-src` is absent
/// (the tree is gitignored; a clone relies on the law tests above).
#[test]
fn rpcs3_ppudisasm_recognizes_every_mnemonic() {
    let path = std::path::PathBuf::from("../../tools/rpcs3-src/rpcs3/Emu/Cell/PPUDisAsm.h");
    if !path.exists() {
        return;
    }
    let src = std::fs::read_to_string(&path).expect("PPUDisAsm.h must be readable");
    let names = VxOp::iter()
        .map(|op| <&'static str>::from(op).to_uppercase())
        .chain(VaOp::iter().map(|op| <&'static str>::from(op).to_uppercase()))
        .chain(Fp63Op::iter().map(|op| <&'static str>::from(op).to_uppercase()))
        .chain(Fp59Op::iter().map(|op| <&'static str>::from(op).to_uppercase()));
    let mut missing = Vec::new();
    for name in names {
        let needle = format!("void {name}(ppu_opcode_t");
        if !src.contains(&needle) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "RPCS3 PPUDisAsm has no method for: {missing:?}"
    );
}
