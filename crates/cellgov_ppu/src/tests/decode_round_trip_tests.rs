//! Decode -> re-encode round trip preserving XO-form Rc and OE bits.

use super::*;

// -- Round-trip tripwire --
//
// Mini-encoder covering the variants that take Rc and OE bits.
// Catches the rldimi-as-rldicl class of mis-route structurally:
// if decode produced the wrong variant, re-encoding picks the
// wrong sub-opcode and the round-trip diverges.
//
// Not a full encoder. Only covers XO/X/M/MD/XS-form integer ops
// and FP (primaries 59/63). Variants outside this set return None.

fn encode(insn: &PpuInstruction) -> Option<u32> {
    let rt = |v: u8| (v as u32 & 0x1F) << 21;
    let ra = |v: u8| (v as u32 & 0x1F) << 16;
    let rb = |v: u8| (v as u32 & 0x1F) << 11;
    let frc = |v: u8| (v as u32 & 0x1F) << 6;
    let p = |v: u32| v << 26;
    let xo_9_oe_rc =
        |xo: u32, oe: bool, rc: bool| -> u32 { ((oe as u32) << 10) | (xo << 1) | (rc as u32) };
    let xo_10_rc = |xo: u32, rc: bool| -> u32 { (xo << 1) | (rc as u32) };

    Some(match *insn {
        // XO-form arithmetic (OE + Rc).
        PpuInstruction::Add {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(266, oe, rc),
        PpuInstruction::Subf {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(40, oe, rc),
        PpuInstruction::Subfc {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(8, oe, rc),
        PpuInstruction::Subfe {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(136, oe, rc),
        PpuInstruction::Neg {
            rt: t,
            ra: a,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | xo_9_oe_rc(104, oe, rc),
        PpuInstruction::Mullw {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(235, oe, rc),
        PpuInstruction::Mulld {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(233, oe, rc),
        PpuInstruction::Adde {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(138, oe, rc),
        PpuInstruction::Addze {
            rt: t,
            ra: a,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | xo_9_oe_rc(202, oe, rc),
        PpuInstruction::Divw {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(491, oe, rc),
        PpuInstruction::Divwu {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(459, oe, rc),
        PpuInstruction::Divd {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(489, oe, rc),
        PpuInstruction::Divdu {
            rt: t,
            ra: a,
            rb: b,
            oe,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(457, oe, rc),

        // Multiply-high family (Rc only, no OE).
        PpuInstruction::Mulhwu {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(11, false, rc),
        PpuInstruction::Mulhw {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(75, false, rc),
        PpuInstruction::Mulhdu {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(9, false, rc),
        PpuInstruction::Mulhd {
            rt: t,
            ra: a,
            rb: b,
            rc,
        } => p(31) | rt(t) | ra(a) | rb(b) | xo_9_oe_rc(73, false, rc),

        // X-form logical (Rc only). RS occupies the RT slot.
        PpuInstruction::Or {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(444, rc),
        PpuInstruction::Orc {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(412, rc),
        PpuInstruction::And {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(28, rc),
        PpuInstruction::Andc {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(60, rc),
        PpuInstruction::Nor {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(124, rc),
        PpuInstruction::Xor {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(316, rc),

        // X-form shifts (Rc).
        PpuInstruction::Slw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(24, rc),
        PpuInstruction::Srw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(536, rc),
        PpuInstruction::Sld {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(27, rc),
        PpuInstruction::Srd {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(539, rc),
        PpuInstruction::Sraw {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(792, rc),
        PpuInstruction::Srad {
            ra: a,
            rs,
            rb: b,
            rc,
        } => p(31) | rt(rs) | ra(a) | rb(b) | xo_10_rc(794, rc),
        PpuInstruction::Srawi { ra: a, rs, sh, rc } => {
            p(31) | rt(rs) | ra(a) | ((sh as u32 & 0x1F) << 11) | xo_10_rc(824, rc)
        }
        // XS-form: SH low 5 bits in bits 11..15, SH high bit at raw bit 1.
        PpuInstruction::Sradi { ra: a, rs, sh, rc } => {
            let sh_lo = (sh as u32 & 0x1F) << 11;
            let sh_hi = ((sh as u32 >> 5) & 1) << 1;
            p(31) | rt(rs) | ra(a) | sh_lo | (413u32 << 2) | sh_hi | (rc as u32)
        }

        // Cntlz/Extsh/Extsb/Extsw (Rc).
        PpuInstruction::Cntlzw { ra: a, rs, rc } => p(31) | rt(rs) | ra(a) | xo_10_rc(26, rc),
        PpuInstruction::Cntlzd { ra: a, rs, rc } => p(31) | rt(rs) | ra(a) | xo_10_rc(58, rc),
        PpuInstruction::Extsh { ra: a, rs, rc } => p(31) | rt(rs) | ra(a) | xo_10_rc(922, rc),
        PpuInstruction::Extsb { ra: a, rs, rc } => p(31) | rt(rs) | ra(a) | xo_10_rc(954, rc),
        PpuInstruction::Extsw { ra: a, rs, rc } => p(31) | rt(rs) | ra(a) | xo_10_rc(986, rc),

        // M-form rotates (Rc).
        PpuInstruction::Rlwimi {
            ra: a,
            rs,
            sh,
            mb,
            me,
            rc,
        } => {
            p(20)
                | rt(rs)
                | ra(a)
                | ((sh as u32 & 0x1F) << 11)
                | ((mb as u32 & 0x1F) << 6)
                | ((me as u32 & 0x1F) << 1)
                | (rc as u32)
        }
        PpuInstruction::Rlwinm {
            ra: a,
            rs,
            sh,
            mb,
            me,
            rc,
        } => {
            p(21)
                | rt(rs)
                | ra(a)
                | ((sh as u32 & 0x1F) << 11)
                | ((mb as u32 & 0x1F) << 6)
                | ((me as u32 & 0x1F) << 1)
                | (rc as u32)
        }
        PpuInstruction::Rlwnm {
            ra: a,
            rs,
            rb: b,
            mb,
            me,
            rc,
        } => {
            p(23)
                | rt(rs)
                | ra(a)
                | rb(b)
                | ((mb as u32 & 0x1F) << 6)
                | ((me as u32 & 0x1F) << 1)
                | (rc as u32)
        }

        // MD-form rotates (Rc + 3-bit sub-opcode).
        PpuInstruction::Rldicl {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => encode_md(rs, a, sh, mb, 0, rc),
        PpuInstruction::Rldicr {
            ra: a,
            rs,
            sh,
            me,
            rc,
        } => encode_md(rs, a, sh, me, 1, rc),
        PpuInstruction::Rldic {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => encode_md(rs, a, sh, mb, 2, rc),
        PpuInstruction::Rldimi {
            ra: a,
            rs,
            sh,
            mb,
            rc,
        } => encode_md(rs, a, sh, mb, 3, rc),

        // dcbz: X-form, no fields beyond RA/RB.
        PpuInstruction::Dcbz { ra: a, rb: b } => p(31) | ra(a) | rb(b) | xo_10_rc(1014, false),

        // FP (Rc preserved, not yet honored). A-form ops store the
        // 5-bit XO (FRC re-encodes separately); X-form ops store the
        // full 10-bit XO, whose top 5 bits coincide with the decoded
        // frc field, so the OR is idempotent either way.
        PpuInstruction::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc: c,
            rc,
        } => p(63) | rt(frt) | ra(fra) | rb(frb) | frc(c) | ((op as u32) << 1) | (rc as u32),
        PpuInstruction::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc: c,
            rc,
        } => p(59) | rt(frt) | ra(fra) | rb(frb) | frc(c) | ((op as u32) << 1) | (rc as u32),

        _ => return None,
    })
}

fn encode_md(rs: u8, ra_val: u8, sh: u8, mask: u8, xo: u32, rc: bool) -> u32 {
    let sh_lo = (sh as u32 & 0x1F) << 11;
    let sh_hi = ((sh as u32 >> 5) & 1) << 1;
    let mask_lo = (mask as u32 & 0x1F) << 6;
    let mask_hi = ((mask as u32 >> 5) & 1) << 5;
    (30u32 << 26)
        | ((rs as u32 & 0x1F) << 21)
        | ((ra_val as u32 & 0x1F) << 16)
        | sh_lo
        | mask_lo
        | mask_hi
        | (xo << 2)
        | sh_hi
        | (rc as u32)
}

#[test]
fn round_trip_preserves_xo_form_rc_and_oe() {
    // Corpus: every combination of Rc and OE where applicable,
    // across the XO-form arithmetic, X-form logical and shift,
    // M-form and MD-form rotates, and FP. Each entry is raw u32.
    // Primary 31, RT=5, RA=6, RB=7 where possible. Dot/oe toggles
    // are the bits most likely to silently drop.
    let xo9_ops = [266u32, 40, 235, 233, 138, 491, 459, 489, 457]; // add,subf,mullw,mulld,adde,divw,divwu,divd,divdu
    let mut corpus: Vec<u32> = Vec::new();
    for &xo in &xo9_ops {
        for oe in [0u32, 1] {
            for rc in [0u32, 1] {
                corpus.push(
                    (31u32 << 26)
                        | (5u32 << 21)
                        | (6u32 << 16)
                        | (7u32 << 11)
                        | (oe << 10)
                        | (xo << 1)
                        | rc,
                );
            }
        }
    }
    // addze / neg: no RB slot.
    for &xo in &[202u32, 104] {
        for oe in [0u32, 1] {
            for rc in [0u32, 1] {
                corpus.push(
                    (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (oe << 10) | (xo << 1) | rc,
                );
            }
        }
    }
    // mulh family: xo_9 only, no OE bit meaningful.
    for &xo in &[11u32, 75, 9, 73] {
        for rc in [0u32, 1] {
            corpus
                .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc);
        }
    }
    // X-form logical + shift (use RB=7).
    for &xo in &[444u32, 412, 28, 60, 124, 316, 24, 536, 27, 539, 792, 794] {
        for rc in [0u32, 1] {
            corpus
                .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc);
        }
    }
    // cntlz + extsb/h/w: reserved RB slot is zero in canonical encodings.
    for &xo in &[26u32, 58, 922, 954, 986] {
        for rc in [0u32, 1] {
            corpus.push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (xo << 1) | rc);
        }
    }
    // srawi: SH in RB slot.
    for rc in [0u32, 1] {
        corpus
            .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (12u32 << 11) | (824u32 << 1) | rc);
    }
    // sradi: XS-form. SH=34 (hi=1, lo=2): sh_lo=2 at bits 11..15, sh_hi=1 at bit 1.
    for rc in [0u32, 1] {
        corpus.push(
            (31u32 << 26)
                | (5u32 << 21)
                | (6u32 << 16)
                | (2u32 << 11)
                | (413u32 << 2)
                | (1u32 << 1)
                | rc,
        );
        // SH=3 (hi=0, lo=3).
        corpus
            .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (3u32 << 11) | (413u32 << 2) | rc);
    }
    // M-form: rlwimi, rlwinm, rlwnm with sh=4, mb=8, me=20.
    for primary in [20u32, 21] {
        for rc in [0u32, 1] {
            corpus.push(
                (primary << 26)
                    | (5u32 << 21)
                    | (6u32 << 16)
                    | (4u32 << 11)
                    | (8u32 << 6)
                    | (20u32 << 1)
                    | rc,
            );
        }
    }
    for rc in [0u32, 1] {
        corpus.push(
            (23u32 << 26)
                | (5u32 << 21)
                | (6u32 << 16)
                | (7u32 << 11)
                | (8u32 << 6)
                | (20u32 << 1)
                | rc,
        );
    }
    // MD-form rotates. mask=33 (hi=1, lo=1), sh=34 (hi=1, lo=2).
    for xo in 0..=3u32 {
        for rc in [0u32, 1] {
            corpus.push(
                (30u32 << 26)
                    | (5u32 << 21)
                    | (6u32 << 16)
                    | (2u32 << 11)
                    | (1u32 << 6)
                    | (1u32 << 5)
                    | (xo << 2)
                    | (1u32 << 1)
                    | rc,
            );
        }
    }
    // dcbz: RA=6, RB=7.
    corpus.push((31u32 << 26) | (6u32 << 16) | (7u32 << 11) | (1014u32 << 1));

    // FP primary 59 and 63: xo=21 (fadd), xo=25 (fmul low 5), Rc=0/1.
    for &primary in &[59u32, 63] {
        for &xo in &[21u32, 50] {
            for rc in [0u32, 1] {
                corpus.push(
                    (primary << 26)
                        | (5u32 << 21)
                        | (6u32 << 16)
                        | (7u32 << 11)
                        | (2u32 << 6)
                        | (xo << 1)
                        | rc,
                );
            }
        }
    }

    assert!(!corpus.is_empty(), "round-trip corpus must not be empty");
    for raw in corpus {
        let decoded =
            decode(raw).unwrap_or_else(|e| panic!("decode failed for {raw:#010x}: {e:?}"));
        let reencoded = encode(&decoded).unwrap_or_else(|| {
            panic!("encoder missing variant for decoded={decoded:?} (raw={raw:#010x})")
        });
        assert_eq!(
            reencoded, raw,
            "round-trip mismatch: raw={raw:#010x} decoded={decoded:?} re-encoded={reencoded:#010x}",
        );
    }
}
