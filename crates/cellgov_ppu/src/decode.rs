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
        39 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stbu { rs, ra, imm })
        }
        44 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Sth { rs, ra, imm })
        }
        45 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Sthu { rs, ra, imm })
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
            let l = ((raw >> 21) & 0x1) as u8;
            let (_, ra, imm) = d_form(raw);
            if l == 0 {
                Ok(PpuInstruction::Cmpwi { bf, ra, imm })
            } else {
                Ok(PpuInstruction::Cmpdi { bf, ra, imm })
            }
        }
        10 => {
            let bf = ((raw >> 23) & 0x7) as u8;
            let l = ((raw >> 21) & 0x1) as u8;
            let (_, ra, imm) = d_form_u(raw);
            if l == 0 {
                Ok(PpuInstruction::Cmplwi { bf, ra, imm })
            } else {
                Ok(PpuInstruction::Cmpldi { bf, ra, imm })
            }
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

        // M-form (primaries 20, 21, 23): 5-bit MB at bits 21..=25 and
        // 5-bit ME at bits 26..=30. Not to be confused with MD-form's
        // 6-bit mask bound (primary 30, split across bits 21..=25 and
        // bit 26).
        20 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let sh = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            let rc = raw & 1 != 0;
            Ok(PpuInstruction::Rlwimi {
                ra,
                rs,
                sh,
                mb,
                me,
                rc,
            })
        }
        21 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let sh = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            let rc = raw & 1 != 0;
            Ok(PpuInstruction::Rlwinm {
                ra,
                rs,
                sh,
                mb,
                me,
                rc,
            })
        }
        23 => {
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let rb = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            let rc = raw & 1 != 0;
            Ok(PpuInstruction::Rlwnm {
                ra,
                rs,
                rb,
                mb,
                me,
                rc,
            })
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
            let rc = raw & 1 != 0;
            if primary == 63 {
                Ok(PpuInstruction::Fp63 {
                    xo,
                    frt,
                    fra,
                    frb,
                    frc,
                    rc,
                })
            } else {
                Ok(PpuInstruction::Fp59 {
                    xo,
                    frt,
                    fra,
                    frb,
                    frc,
                    rc,
                })
            }
        }

        17 => {
            // SC-form (Book I Sec. 3.3.1, p. 35): LEV occupies PPC
            // bits 20..=26 (7 bits). In Rust LSB-0 that is bits 5..=11.
            let lev = ((raw >> 5) & 0x7F) as u8;
            Ok(PpuInstruction::Sc { lev })
        }

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
        // vsldoi puts a 4-bit byte shift (SHB) in the vc slot, with bit
        // 21 reserved. Other VA-form ops treat the slot as a register.
        if xo_6 == 0x2c {
            return Ok(PpuInstruction::Vsldoi {
                vt,
                va,
                vb,
                shb: vc & 0xF,
            });
        }
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
    let rc = raw & 1 != 0;

    match xo {
        0 => Ok(PpuInstruction::Rldicl {
            ra,
            rs,
            sh,
            mb: mask,
            rc,
        }),
        1 => Ok(PpuInstruction::Rldicr {
            ra,
            rs,
            sh,
            me: mask,
            rc,
        }),
        2 => Ok(PpuInstruction::Rldic {
            ra,
            rs,
            sh,
            mb: mask,
            rc,
        }),
        3 => Ok(PpuInstruction::Rldimi {
            ra,
            rs,
            sh,
            mb: mask,
            rc,
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
    let rc = raw & 1 != 0;
    let oe = (raw >> 10) & 1 != 0;

    match xo_9 {
        266 => return Ok(PpuInstruction::Add { rt, ra, rb, oe, rc }),
        40 => return Ok(PpuInstruction::Subf { rt, ra, rb, oe, rc }),
        8 => return Ok(PpuInstruction::Subfc { rt, ra, rb, oe, rc }),
        136 => return Ok(PpuInstruction::Subfe { rt, ra, rb, oe, rc }),
        104 => return Ok(PpuInstruction::Neg { rt, ra, oe, rc }),
        235 => return Ok(PpuInstruction::Mullw { rt, ra, rb, oe, rc }),
        11 => return Ok(PpuInstruction::Mulhwu { rt, ra, rb, rc }),
        9 => return Ok(PpuInstruction::Mulhdu { rt, ra, rb, rc }),
        73 => return Ok(PpuInstruction::Mulhd { rt, ra, rb, rc }),
        75 => return Ok(PpuInstruction::Mulhw { rt, ra, rb, rc }),
        138 => return Ok(PpuInstruction::Adde { rt, ra, rb, oe, rc }),
        202 => return Ok(PpuInstruction::Addze { rt, ra, oe, rc }),
        491 => return Ok(PpuInstruction::Divw { rt, ra, rb, oe, rc }),
        459 => return Ok(PpuInstruction::Divwu { rt, ra, rb, oe, rc }),
        489 => return Ok(PpuInstruction::Divd { rt, ra, rb, oe, rc }),
        457 => return Ok(PpuInstruction::Divdu { rt, ra, rb, oe, rc }),
        233 => return Ok(PpuInstruction::Mulld { rt, ra, rb, oe, rc }),
        _ => {}
    }

    match xo_10 {
        444 => return Ok(PpuInstruction::Or { ra, rs: rt, rb, rc }),
        412 => return Ok(PpuInstruction::Orc { ra, rs: rt, rb, rc }),
        28 => return Ok(PpuInstruction::And { ra, rs: rt, rb, rc }),
        60 => return Ok(PpuInstruction::Andc { ra, rs: rt, rb, rc }),
        124 => return Ok(PpuInstruction::Nor { ra, rs: rt, rb, rc }),
        316 => return Ok(PpuInstruction::Xor { ra, rs: rt, rb, rc }),

        24 => return Ok(PpuInstruction::Slw { ra, rs: rt, rb, rc }),
        536 => return Ok(PpuInstruction::Srw { ra, rs: rt, rb, rc }),

        27 => return Ok(PpuInstruction::Sld { ra, rs: rt, rb, rc }),
        539 => return Ok(PpuInstruction::Srd { ra, rs: rt, rb, rc }),
        792 => return Ok(PpuInstruction::Sraw { ra, rs: rt, rb, rc }),
        794 => return Ok(PpuInstruction::Srad { ra, rs: rt, rb, rc }),

        824 => {
            let sh = rb;
            return Ok(PpuInstruction::Srawi { ra, rs: rt, sh, rc });
        }
        // sradi is XS-form: XO(9)=413 occupies bits 21..29, raw bit 30
        // holds the SH high bit, raw bit 31 is Rc. Extracting as a
        // 10-bit XO captures (XO(9) << 1) | SH_hi, so both 826 (SH_hi=0)
        // and 827 (SH_hi=1) are sradi.
        826 | 827 => {
            let sh_hi = ((raw >> 1) & 0x1) as u8;
            let sh = rb | (sh_hi << 5);
            return Ok(PpuInstruction::Sradi { ra, rs: rt, sh, rc });
        }

        26 => return Ok(PpuInstruction::Cntlzw { ra, rs: rt, rc }),
        58 => return Ok(PpuInstruction::Cntlzd { ra, rs: rt, rc }),

        922 => return Ok(PpuInstruction::Extsh { ra, rs: rt, rc }),
        954 => return Ok(PpuInstruction::Extsb { ra, rs: rt, rc }),
        986 => return Ok(PpuInstruction::Extsw { ra, rs: rt, rc }),

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
        // SPR / TBR half-swap (Book I Sec. 3.3.15, p. 84; Book II
        // Sec. 4.1, p. 30). The encoded register number is split
        // across the RA and RB slots, and the halves are reversed:
        //   rb (bits 11..=15) = SPR/TBR low 5 bits
        //   ra (bits 16..=20) = SPR/TBR high 5 bits
        // So `spr_raw = (rb << 5) | ra`. Do not "simplify" the
        // shift direction; the apparent reversal is the encoding.
        339 => {
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            return match spr_raw {
                8 => Ok(PpuInstruction::Mflr { rt }),
                9 => Ok(PpuInstruction::Mfctr { rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            };
        }
        371 => {
            let tbr = ((rb as u16) << 5) | (ra as u16);
            return match tbr {
                268 => Ok(PpuInstruction::Mftb { rt }),
                269 => Ok(PpuInstruction::Mftbu { rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            };
        }
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
                rc: false,
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
                rc: false,
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
                rc: false,
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
                oe: false,
                rc: false,
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
                rc: false,
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
                rc: false,
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
                rc: false,
            }
        );
    }

    #[test]
    fn addze_decodes() {
        // addze r0, r0 -> XO(9)=202 -> 0x7C00_0194.
        let insn = decode(0x7C00_0194).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Addze {
                rt: 0,
                ra: 0,
                oe: false,
                rc: false,
            }
        );
    }

    #[test]
    fn cntlzd_decodes() {
        // cntlzd r0, r11 -> XO(10)=58 -> 0x7D60_0074.
        let insn = decode(0x7D60_0074).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Cntlzd {
                ra: 0,
                rs: 11,
                rc: false,
            }
        );
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
                rc: false,
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

    #[test]
    fn cmpi_l_bit_selects_cmpdi() {
        // cmpdi cr0, r3, 0 -> primary 11, BF=0, L=1, RA=3, imm=0 -> 0x2C23_0000
        let insn = decode(0x2C23_0000).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Cmpdi {
                bf: 0,
                ra: 3,
                imm: 0,
            }
        );
    }

    #[test]
    fn cmpi_l_bit_zero_is_cmpwi() {
        // cmpwi cr0, r3, 0 -> primary 11, L=0 -> 0x2C03_0000
        let insn = decode(0x2C03_0000).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Cmpwi {
                bf: 0,
                ra: 3,
                imm: 0,
            }
        );
    }

    #[test]
    fn cmpli_l_bit_selects_cmpldi() {
        // cmpldi cr0, r3, 0 -> primary 10, L=1 -> 0x2823_0000
        let insn = decode(0x2823_0000).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Cmpldi {
                bf: 0,
                ra: 3,
                imm: 0,
            }
        );
    }

    #[test]
    fn stbu_decodes_with_update() {
        // stbu r6, -4(r1) -> primary 39, RS=6, RA=1, D=-4 -> 0x9CC1_FFFC
        let insn = decode(0x9CC1_FFFC).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Stbu {
                rs: 6,
                ra: 1,
                imm: -4,
            }
        );
    }

    #[test]
    fn sthu_decodes_with_update() {
        // sthu r5, -8(r1) -> primary 45, RS=5, RA=1, D=-8 -> 0xB4A1_FFF8
        let insn = decode(0xB4A1_FFF8).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Sthu {
                rs: 5,
                ra: 1,
                imm: -8,
            }
        );
    }

    #[test]
    fn rldic_decodes_from_xo_2() {
        // Build rldic r5, r4, SH=4, MB=32 manually.
        // primary=30, RS=4, RA=5, sh_lo=4 (SH&0x1F), mb_lo=32&0x1F=0,
        // mb_hi=(32>>5)&1 = 1, xo=2, sh_hi=0, Rc=0.
        let raw: u32 =
            (30 << 26) | (4u32 << 21) | (5u32 << 16) | (4u32 << 11) | (1u32 << 5) | (2u32 << 2);
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rldic {
                ra: 5,
                rs: 4,
                sh: 4,
                mb: 32,
                rc: false,
            }
        );
    }

    #[test]
    fn rldimi_decodes_from_xo_3() {
        // rldimi r5, r4, SH=16, MB=0.
        // primary=30, RS=4, RA=5, sh_lo=16, mb_lo=0, mb_hi=0, xo=3.
        let raw: u32 = (30 << 26) | (4u32 << 21) | (5u32 << 16) | (16u32 << 11) | (3u32 << 2);
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rldimi {
                ra: 5,
                rs: 4,
                sh: 16,
                mb: 0,
                rc: false,
            }
        );
    }

    #[test]
    fn xo_794_decodes_as_srad_not_sraw() {
        // primary=31, RT=5, RA=6, RB=7, XO(10)=794, Rc=0.
        let raw: u32 = (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (794u32 << 1);
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Srad {
                ra: 6,
                rs: 5,
                rb: 7,
                rc: false,
            }
        );
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
    fn add_dot_decodes_with_rc_set() {
        // add. r3, r4, r5 -> primary 31, XO(9)=266, Rc=1.
        let raw: u32 =
            (31u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (266u32 << 1) | 1;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Add {
                rt: 3,
                ra: 4,
                rb: 5,
                oe: false,
                rc: true,
            }
        );
    }

    #[test]
    fn addo_decodes_with_oe_set() {
        // addo r3, r4, r5 -> primary 31, RT=3, RA=4, RB=5, OE=1, XO(9)=266, Rc=0.
        let raw: u32 = (31u32 << 26)
            | (3u32 << 21)
            | (4u32 << 16)
            | (5u32 << 11)
            | (1u32 << 10)
            | (266u32 << 1);
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Add {
                rt: 3,
                ra: 4,
                rb: 5,
                oe: true,
                rc: false,
            }
        );
    }

    #[test]
    fn or_dot_decodes_with_rc_set() {
        // or. r3, r4, r5 -> primary 31, XO(10)=444, Rc=1.
        let raw: u32 =
            (31u32 << 26) | (4u32 << 21) | (3u32 << 16) | (5u32 << 11) | (444u32 << 1) | 1;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Or {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: true,
            }
        );
    }

    #[test]
    fn rldicl_dot_decodes_with_rc_set() {
        // rldicl. r5, r4, sh=0, mb=61, Rc=1.
        let raw: u32 = (30u32 << 26) | (4u32 << 21) | (5u32 << 16) | (29u32 << 6) | 1;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Rldicl {
                ra: 5,
                rs: 4,
                sh: 0,
                mb: 29,
                rc: true,
            }
        );
    }

    #[test]
    fn vsldoi_decodes_with_shb_field() {
        // vsldoi v3, v1, v2, 4 -> primary 4, VT=3, VA=1, VB=2,
        // vc field holds SHB=4 in its low nibble, xo_6=0x2C.
        // Layout: primary=4, VT=3, VA=1, VB=2, shb=4 in bits 21..25
        // (vc slot), xo_6 in bits 0..5.
        let raw: u32 =
            (4u32 << 26) | (3u32 << 21) | (1u32 << 16) | (2u32 << 11) | (4u32 << 6) | 0x2C;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Vsldoi {
                vt: 3,
                va: 1,
                vb: 2,
                shb: 4,
            }
        );
    }

    // -- Round-trip tripwire --
    //
    // Mini-encoder covering the variants touched by the Rc/OE slice.
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

            // FP (Rc preserved, not yet honored).
            PpuInstruction::Fp63 {
                xo,
                frt,
                fra,
                frb,
                frc: c,
                rc,
            } => p(63) | rt(frt) | ra(fra) | rb(frb) | frc(c) | ((xo as u32) << 1) | (rc as u32),
            PpuInstruction::Fp59 {
                xo,
                frt,
                fra,
                frb,
                frc: c,
                rc,
            } => p(59) | rt(frt) | ra(fra) | rb(frb) | frc(c) | ((xo as u32) << 1) | (rc as u32),

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
                corpus.push(
                    (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc,
                );
            }
        }
        // X-form logical + shift (use RB=7).
        for &xo in &[444u32, 412, 28, 60, 124, 316, 24, 536, 27, 539, 792, 794] {
            for rc in [0u32, 1] {
                corpus.push(
                    (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc,
                );
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
            corpus.push(
                (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (12u32 << 11) | (824u32 << 1) | rc,
            );
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
            corpus.push(
                (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (3u32 << 11) | (413u32 << 2) | rc,
            );
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
}
