//! Pure 32-bit-word to `PpuInstruction` decoder.
//!
//! PPC instructions are fixed-width 32-bit big-endian. Primary opcode
//! occupies bits 0-5; extended opcodes live at bits 21-30 (XO-form)
//! or other positions depending on form. Unknown encodings produce
//! `PpuDecodeError::Unsupported(raw)` -- the decoder never panics.

use crate::instruction::{PpuDecodeError, PpuInstruction};

/// Extract D-form fields as `(rt/rs, ra, signed imm16)`.
#[inline]
fn d_form(raw: u32) -> (u8, u8, i16) {
    (
        ((raw >> 21) & 0x1F) as u8,
        ((raw >> 16) & 0x1F) as u8,
        (raw & 0xFFFF) as i16,
    )
}

/// Extract D-form fields as `(rt/rs, ra, unsigned imm16)`.
#[inline]
fn d_form_u(raw: u32) -> (u8, u8, u16) {
    (
        ((raw >> 21) & 0x1F) as u8,
        ((raw >> 16) & 0x1F) as u8,
        (raw & 0xFFFF) as u16,
    )
}

/// Extract X-form fields as `(rt/rs, ra, rb)`.
#[inline]
fn x_form(raw: u32) -> (u8, u8, u8) {
    (
        ((raw >> 21) & 0x1F) as u8,
        ((raw >> 16) & 0x1F) as u8,
        ((raw >> 11) & 0x1F) as u8,
    )
}

/// Decode a 32-bit PPC instruction word.
///
/// # Errors
///
/// Returns `PpuDecodeError::Unsupported(raw)` for any encoding the
/// decoder does not recognise.
pub fn decode(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let primary = (raw >> 26) & 0x3F;

    match primary {
        4 => decode_vx(raw),

        7 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Mulli { rt, ra, imm })
        }
        8 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Subfic { rt, ra, imm })
        }
        12 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Addic { rt, ra, imm })
        }

        32 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lwz { rt, ra, imm })
        }
        33 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lwzu { rt, ra, imm })
        }
        34 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lbz { rt, ra, imm })
        }
        35 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lbzu { rt, ra, imm })
        }
        40 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lhz { rt, ra, imm })
        }
        41 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lhzu { rt, ra, imm })
        }
        42 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lha { rt, ra, imm })
        }
        58 => {
            // DS-form: low 2 bits select between ld/ldu/lwa.
            let sub = raw & 0x3;
            let (rt, ra, _) = d_form(raw);
            let imm = (raw & 0xFFFC) as i16;
            match sub {
                0 => Ok(PpuInstruction::Ld { rt, ra, imm }),
                1 => Ok(PpuInstruction::Ldu { rt, ra, imm }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            }
        }

        36 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stw { rs, ra, imm })
        }
        37 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stwu { rs, ra, imm })
        }
        38 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stb { rs, ra, imm })
        }
        // stbu decodes as Stb; the update-to-ra semantic is dropped.
        39 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stb { rs, ra, imm })
        }
        44 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Sth { rs, ra, imm })
        }
        62 => {
            // DS-form: low 2 bits select between std/stdu.
            let sub = raw & 0x3;
            let (rs, ra, _) = d_form(raw);
            let imm = (raw & 0xFFFC) as i16;
            match sub {
                0 => Ok(PpuInstruction::Std { rs, ra, imm }),
                1 => Ok(PpuInstruction::Stdu { rs, ra, imm }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            }
        }

        14 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Addi { rt, ra, imm })
        }
        15 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Addis { rt, ra, imm })
        }
        24 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::Ori { ra, rs, imm })
        }
        25 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::Oris { ra, rs, imm })
        }
        26 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::Xori { ra, rs, imm })
        }
        27 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::Xoris { ra, rs, imm })
        }
        28 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::AndiDot { ra, rs, imm })
        }

        11 => {
            let bf = ((raw >> 23) & 0x7) as u8;
            let (_, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Cmpwi { bf, ra, imm })
        }
        10 => {
            let bf = ((raw >> 23) & 0x7) as u8;
            let (_, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::Cmplwi { bf, ra, imm })
        }

        18 => {
            // I-form LI field is bits 6..29 shifted left 2; sign-extend
            // from bit 25 (the MSB of LI after the shift).
            let li = raw & 0x03FF_FFFC;
            let offset = if li & 0x0200_0000 != 0 {
                (li | 0xFC00_0000) as i32
            } else {
                li as i32
            };
            let aa = raw & 2 != 0;
            let link = raw & 1 != 0;
            Ok(PpuInstruction::B { offset, aa, link })
        }

        16 => {
            let bo = ((raw >> 21) & 0x1F) as u8;
            let bi = ((raw >> 16) & 0x1F) as u8;
            let bd = (raw & 0xFFFC) as i16;
            let link = raw & 1 != 0;
            Ok(PpuInstruction::Bc {
                bo,
                bi,
                offset: bd,
                link,
            })
        }

        19 => decode_xl(raw),

        20 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let sh = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            Ok(PpuInstruction::Rlwimi { ra, rs, sh, mb, me })
        }
        21 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let sh = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            Ok(PpuInstruction::Rlwinm { ra, rs, sh, mb, me })
        }
        23 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let rb = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            Ok(PpuInstruction::Rlwnm { ra, rs, rb, mb, me })
        }
        30 => decode_md(raw),

        31 => decode_x31(raw),

        48 => {
            let (frt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lfs { frt, ra, imm })
        }
        50 => {
            let (frt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lfd { frt, ra, imm })
        }
        52 => {
            let (frs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stfs { frs, ra, imm })
        }
        53 => {
            let (frs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stfsu { frs, ra, imm })
        }
        54 => {
            let (frs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stfd { frs, ra, imm })
        }
        55 => {
            let (frs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stfdu { frs, ra, imm })
        }

        63 | 59 => {
            let (frt, fra, frb) = x_form(raw);
            let frc = ((raw >> 6) & 0x1F) as u8;
            let xo = ((raw >> 1) & 0x3FF) as u16;
            if primary == 63 {
                Ok(PpuInstruction::Fp63 {
                    xo,
                    frt,
                    fra,
                    frb,
                    frc,
                })
            } else {
                Ok(PpuInstruction::Fp59 {
                    xo,
                    frt,
                    fra,
                    frb,
                    frc,
                })
            }
        }

        17 => Ok(PpuInstruction::Sc),

        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 4: VA-form (XO bits 0..5 in 0x20..=0x2f) or
/// VX-form (XO bits 21..31, four register operands).
fn decode_vx(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let vt = ((raw >> 21) & 0x1F) as u8;
    let va = ((raw >> 16) & 0x1F) as u8;
    let vb = ((raw >> 11) & 0x1F) as u8;
    let vc = ((raw >> 6) & 0x1F) as u8;
    let xo_11 = raw & 0x7FF;
    let xo_6 = (raw & 0x3F) as u8;

    if let 0x20..=0x2f = xo_6 {
        return Ok(PpuInstruction::Va {
            xo: xo_6,
            vt,
            va,
            vb,
            vc,
        });
    }

    match xo_11 {
        0x4c4 => Ok(PpuInstruction::Vxor { vt, va, vb }),
        _ => Ok(PpuInstruction::Vx {
            xo: xo_11 as u16,
            vt,
            va,
            vb,
        }),
    }
}

/// Decode primary opcode 30 (MD-form: rldicl, rldicr, rldic, rldimi).
///
/// MD-form splits the 6-bit SH across bits 16..20 (low) and bit 30
/// (high); the 6-bit mask bound splits across bits 21..25 (low) and
/// bit 26 (high). Sub-opcode lives in bits 27..29.
fn decode_md(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let rs = ((raw >> 21) & 0x1F) as u8;
    let ra = ((raw >> 16) & 0x1F) as u8;
    let sh_lo = ((raw >> 11) & 0x1F) as u8;
    let mask_lo = ((raw >> 6) & 0x1F) as u8;
    let mask_hi = ((raw >> 5) & 0x1) as u8;
    let xo = ((raw >> 2) & 0x7) as u8;
    let sh_hi = ((raw >> 1) & 0x1) as u8;
    let sh = (sh_hi << 5) | sh_lo;
    let mask = (mask_hi << 5) | mask_lo;

    match xo {
        0 => Ok(PpuInstruction::Rldicl {
            ra,
            rs,
            sh,
            mb: mask,
        }),
        1 => Ok(PpuInstruction::Rldicr {
            ra,
            rs,
            sh,
            me: mask,
        }),
        // rldic and rldimi are approximated as Rldicl; the full
        // clear-both-sides and insert-merge semantics are not modelled.
        2 | 3 => Ok(PpuInstruction::Rldicl {
            ra,
            rs,
            sh,
            mb: mask,
        }),
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 19 (XL-form: bclr, bcctr, isync, ...).
fn decode_xl(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let xo = (raw >> 1) & 0x3FF;
    let bo = ((raw >> 21) & 0x1F) as u8;
    let bi = ((raw >> 16) & 0x1F) as u8;
    let link = raw & 1 != 0;

    match xo {
        16 => Ok(PpuInstruction::Bclr { bo, bi, link }),
        528 => Ok(PpuInstruction::Bcctr { bo, bi, link }),
        // isync decodes as `ori 0,0,0` (a nop under the deterministic model).
        150 => Ok(PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        }),
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 31 (X-form and XO-form).
///
/// XO-form uses a 9-bit extended opcode at bits 22..30; X-form uses
/// a 10-bit extended opcode at bits 21..30. The 9-bit match runs
/// first; on miss the 10-bit match takes over.
fn decode_x31(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let xo_10 = (raw >> 1) & 0x3FF;
    let xo_9 = (raw >> 1) & 0x1FF;
    let (rt, ra, rb) = x_form(raw);

    match xo_9 {
        266 => return Ok(PpuInstruction::Add { rt, ra, rb }),
        40 => return Ok(PpuInstruction::Subf { rt, ra, rb }),
        8 => return Ok(PpuInstruction::Subfc { rt, ra, rb }),
        136 => return Ok(PpuInstruction::Subfe { rt, ra, rb }),
        104 => return Ok(PpuInstruction::Neg { rt, ra }),
        235 => return Ok(PpuInstruction::Mullw { rt, ra, rb }),
        11 => return Ok(PpuInstruction::Mulhwu { rt, ra, rb }),
        9 => return Ok(PpuInstruction::Mulhdu { rt, ra, rb }),
        73 => return Ok(PpuInstruction::Mulhd { rt, ra, rb }),
        75 => return Ok(PpuInstruction::Mulhw { rt, ra, rb }),
        138 => return Ok(PpuInstruction::Adde { rt, ra, rb }),
        202 => return Ok(PpuInstruction::Addze { rt, ra }),
        491 => return Ok(PpuInstruction::Divw { rt, ra, rb }),
        459 => return Ok(PpuInstruction::Divwu { rt, ra, rb }),
        489 => return Ok(PpuInstruction::Divd { rt, ra, rb }),
        457 => return Ok(PpuInstruction::Divdu { rt, ra, rb }),
        233 => return Ok(PpuInstruction::Mulld { rt, ra, rb }),
        _ => {}
    }

    match xo_10 {
        444 => return Ok(PpuInstruction::Or { ra, rs: rt, rb }),
        412 => return Ok(PpuInstruction::Orc { ra, rs: rt, rb }),
        28 => return Ok(PpuInstruction::And { ra, rs: rt, rb }),
        60 => return Ok(PpuInstruction::Andc { ra, rs: rt, rb }),
        124 => return Ok(PpuInstruction::Nor { ra, rs: rt, rb }),
        316 => return Ok(PpuInstruction::Xor { ra, rs: rt, rb }),

        24 => return Ok(PpuInstruction::Slw { ra, rs: rt, rb }),
        536 => return Ok(PpuInstruction::Srw { ra, rs: rt, rb }),

        27 => return Ok(PpuInstruction::Sld { ra, rs: rt, rb }),
        539 => return Ok(PpuInstruction::Srd { ra, rs: rt, rb }),
        // Power ISA assigns XO=792 to sraw and XO=794 to srad. The 794
        // and 827 entries below are legacy decodes that predate ISA
        // alignment and remain until the instruction-coverage tests
        // are re-audited.
        792 => return Ok(PpuInstruction::Sraw { ra, rs: rt, rb }),
        794 => return Ok(PpuInstruction::Sraw { ra, rs: rt, rb }),
        827 => return Ok(PpuInstruction::Srad { ra, rs: rt, rb }),

        824 => {
            let sh = rb;
            return Ok(PpuInstruction::Srawi { ra, rs: rt, sh });
        }
        // sradi is XS-form with XO(10)=826; 827 is srad (register-shift).
        826 => {
            let sh = rb;
            return Ok(PpuInstruction::Sradi { ra, rs: rt, sh });
        }

        26 => return Ok(PpuInstruction::Cntlzw { ra, rs: rt }),
        58 => return Ok(PpuInstruction::Cntlzd { ra, rs: rt }),

        922 => return Ok(PpuInstruction::Extsh { ra, rs: rt }),
        954 => return Ok(PpuInstruction::Extsb { ra, rs: rt }),
        986 => return Ok(PpuInstruction::Extsw { ra, rs: rt }),

        23 => return Ok(PpuInstruction::Lwzx { rt, ra, rb }),
        87 => return Ok(PpuInstruction::Lbzx { rt, ra, rb }),
        21 => return Ok(PpuInstruction::Ldx { rt, ra, rb }),
        279 => return Ok(PpuInstruction::Lhzx { rt, ra, rb }),

        // Cell BE PPU unaligned-vector loads.
        519 => return Ok(PpuInstruction::Lvlx { vt: rt, ra, rb }),
        583 => return Ok(PpuInstruction::Lvrx { vt: rt, ra, rb }),

        84 => return Ok(PpuInstruction::Ldarx { rt, ra, rb }),
        214 => return Ok(PpuInstruction::Stdcx { rs: rt, ra, rb }),
        20 => return Ok(PpuInstruction::Lwarx { rt, ra, rb }),
        150 => return Ok(PpuInstruction::Stwcx { rs: rt, ra, rb }),

        151 => return Ok(PpuInstruction::Stwx { rs: rt, ra, rb }),
        149 => return Ok(PpuInstruction::Stdx { rs: rt, ra, rb }),
        181 => return Ok(PpuInstruction::Stdux { rs: rt, ra, rb }),
        215 => return Ok(PpuInstruction::Stbx { rs: rt, ra, rb }),

        // stfiwx reuses the RT slot for FRS.
        983 => return Ok(PpuInstruction::Stfiwx { frs: rt, ra, rb }),

        103 => {
            return Ok(PpuInstruction::Vx {
                xo: 103,
                vt: rt,
                va: ra,
                vb: rb,
            })
        }
        231 => return Ok(PpuInstruction::Stvx { vs: rt, ra, rb }),

        0 => {
            let bf = rt >> 2;
            let l_bit = rt & 1;
            return if l_bit == 0 {
                Ok(PpuInstruction::Cmpw { bf, ra, rb })
            } else {
                Ok(PpuInstruction::Cmpd { bf, ra, rb })
            };
        }
        32 => {
            let bf = rt >> 2;
            let l_bit = rt & 1;
            return if l_bit == 0 {
                Ok(PpuInstruction::Cmplw { bf, ra, rb })
            } else {
                Ok(PpuInstruction::Cmpld { bf, ra, rb })
            };
        }

        19 => return Ok(PpuInstruction::Mfcr { rt }),
        144 => {
            // mtcrf CRM (FXM field) lives at bits 12..19.
            let crm = ((raw >> 12) & 0xFF) as u8;
            return Ok(PpuInstruction::Mtcrf { rs: rt, crm });
        }
        339 => {
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            return match spr_raw {
                8 => Ok(PpuInstruction::Mflr { rt }),
                9 => Ok(PpuInstruction::Mfctr { rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            };
        }
        371 => return Ok(PpuInstruction::Mftb { rt }),
        467 => {
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            return match spr_raw {
                8 => Ok(PpuInstruction::Mtlr { rs: rt }),
                9 => Ok(PpuInstruction::Mtctr { rs: rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            };
        }

        // Cache and memory-barrier hints: dcbst(54), dcbf(86), dcbt(278),
        // sync/lwsync(598), dcbtst(854), icbi(982), dcbz(1014). The
        // deterministic model collapses all to a nop.
        86 | 54 | 278 | 598 | 854 | 982 | 1014 => {
            return Ok(PpuInstruction::Ori {
                ra: 0,
                rs: 0,
                imm: 0,
            })
        }

        _ => {}
    }

    Err(PpuDecodeError::Unsupported(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_returns_error() {
        let result = decode(0x0800_0000);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PpuDecodeError::Unsupported(0x0800_0000)
        );
    }

    #[test]
    fn li_decodes_as_addi_ra0() {
        // li r3, 42 -> addi r3, r0, 42 -> 0x3860002A
        let raw = 0x3860_002A;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Addi {
                rt: 3,
                ra: 0,
                imm: 42
            }
        );
    }

    #[test]
    fn sc_decodes() {
        // sc -> primary opcode 17 -> 0x44000002
        let raw = 0x4400_0002;
        let insn = decode(raw).unwrap();
        assert_eq!(insn, PpuInstruction::Sc);
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
    fn oris_decodes() {
        // oris r2, r2, 3 -> 0x64420003
        let insn = decode(0x6442_0003).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Oris {
                ra: 2,
                rs: 2,
                imm: 3
            }
        );
    }

    #[test]
    fn stwu_decodes() {
        // stwu r1, -128(r1) -> 0x9421FF80
        let insn = decode(0x9421_FF80).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stwu {
                rs: 1,
                ra: 1,
                imm: -128,
            }
        );
    }

    #[test]
    fn stdu_decodes() {
        // stdu r1, -112(r1) -> 0xF821FF91 (sub-opcode 1)
        let insn = decode(0xF821_FF91).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stdu {
                rs: 1,
                ra: 1,
                imm: -112,
            }
        );
    }

    #[test]
    fn rldicl_clrldi_decodes() {
        // clrldi r9, r3, 61 -> rldicl r9, r3, 0, 61 -> 0x78690760
        let insn = decode(0x7869_0760).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rldicl {
                ra: 9,
                rs: 3,
                sh: 0,
                mb: 61,
            }
        );
    }

    #[test]
    fn rldicr_sldi_decodes() {
        // sldi r9, r3, 4 -> rldicr r9, r3, 4, 59 -> 0x786926E4
        let insn = decode(0x7869_26E4).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rldicr {
                ra: 9,
                rs: 3,
                sh: 4,
                me: 59,
            }
        );
    }

    #[test]
    fn sth_decodes() {
        // sth r6, -24(r1) -> 0xb0c1ffe8
        let insn = decode(0xb0c1_ffe8).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Sth {
                rs: 6,
                ra: 1,
                imm: -24,
            }
        );
    }

    #[test]
    fn vxor_clears_vector_register() {
        // vxor v0, v0, v0 -> 0x100004C4
        let insn = decode(0x1000_04C4).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Vxor {
                vt: 0,
                va: 0,
                vb: 0,
            }
        );
    }

    #[test]
    fn ldu_decodes_with_negative_ds_offset() {
        // ldu r7, -8(r4): DS=-2 sign-extended through the shift-left-2.
        let insn = decode(0xE8E4_FFF9).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Ldu {
                rt: 7,
                ra: 4,
                imm: -8,
            }
        );
    }

    #[test]
    fn ld_still_decodes_with_sub_zero() {
        // ld r3, 0(r4): primary-58 sub=0 must map to Ld, not Ldu.
        let insn = decode(0xE864_0000).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Ld {
                rt: 3,
                ra: 4,
                imm: 0
            }
        );
    }

    #[test]
    fn rlwnm_decodes() {
        // rlwnm r0, r0, r8, 0, 31 -> 0x5C00_403E.
        let insn = decode(0x5C00_403E).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rlwnm {
                ra: 0,
                rs: 0,
                rb: 8,
                mb: 0,
                me: 31,
            }
        );
    }

    #[test]
    fn adde_decodes() {
        // adde r3, r0, r29 -> XO(9)=138 -> 0x7C60_E914.
        let insn = decode(0x7C60_E914).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Adde {
                rt: 3,
                ra: 0,
                rb: 29,
            }
        );
    }

    #[test]
    fn mulhdu_decodes() {
        // mulhdu r0, r0, r11: opcode 31, RT=0, RA=0, RB=11, XO=9 -> 0x7C005812
        let insn = decode(0x7C00_5812).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Mulhdu {
                rt: 0,
                ra: 0,
                rb: 11,
            }
        );
    }

    #[test]
    fn lbzu_decodes() {
        // lbzu r0, 1(r9) -> opcode 35, RT=0, RA=9, D=1 -> 0x8C090001
        let insn = decode(0x8C09_0001).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Lbzu {
                rt: 0,
                ra: 9,
                imm: 1,
            }
        );
    }

    #[test]
    fn mr_decodes_as_or_rb_eq_rs() {
        // mr r31, r3 -> or r31, r3, r3 -> 0x7C7F1B78
        let insn = decode(0x7C7F_1B78).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Or {
                ra: 31,
                rs: 3,
                rb: 3,
            }
        );
    }

    #[test]
    fn orc_decodes() {
        // orc r0, r11, r28 -> XO(10)=412 -> 0x7D60_E338.
        let insn = decode(0x7D60_E338).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Orc {
                ra: 0,
                rs: 11,
                rb: 28,
            }
        );
    }

    #[test]
    fn addze_decodes() {
        // addze r0, r0 -> XO(9)=202 -> 0x7C00_0194.
        let insn = decode(0x7C00_0194).unwrap();
        assert_eq!(insn, PpuInstruction::Addze { rt: 0, ra: 0 });
    }

    #[test]
    fn cntlzd_decodes() {
        // cntlzd r0, r11 -> XO(10)=58 -> 0x7D60_0074.
        let insn = decode(0x7D60_0074).unwrap();
        assert_eq!(insn, PpuInstruction::Cntlzd { ra: 0, rs: 11 });
    }

    #[test]
    fn stfsu_decodes() {
        // stfsu f13, 8(r8) -> primary 53 -> 0xD5A8_0008.
        let insn = decode(0xD5A8_0008).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stfsu {
                frs: 13,
                ra: 8,
                imm: 8,
            }
        );
    }

    #[test]
    fn stfdu_decodes() {
        // stfdu f1, -8(r1) -> primary 55, FRS=1, RA=1, D=-8 -> 0xDC21_FFF8
        let insn = decode(0xDC21_FFF8).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stfdu {
                frs: 1,
                ra: 1,
                imm: -8,
            }
        );
    }

    #[test]
    fn mulhw_decodes() {
        // mulhw r0, r0, r9 -> XO(9)=75 -> 0x7C00_4896.
        let insn = decode(0x7C00_4896).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Mulhw {
                rt: 0,
                ra: 0,
                rb: 9,
            }
        );
    }

    #[test]
    fn stfiwx_decodes() {
        // stfiwx f13, r0, r9 -> XO(10)=983 -> 0x7DA0_4FAE.
        let insn = decode(0x7DA0_4FAE).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stfiwx {
                frs: 13,
                ra: 0,
                rb: 9,
            }
        );
    }
}
