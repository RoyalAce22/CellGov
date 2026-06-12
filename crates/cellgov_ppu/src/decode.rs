//! Pure 32-bit-word to `PpuInstruction` decoder.
//!
//! PPC instructions are fixed-width 32-bit big-endian. Primary opcode
//! occupies bits 0-5; extended opcodes live at bits 21-30 (XO-form)
//! or other positions depending on form. Encodings the decoder
//! cannot turn into a variant are rejected via
//! [`PpuDecodeError`]: hits in [`crate::instruction::known_encodings`]
//! become [`PpuDecodeError::DecoderArmUnimplemented`] with the
//! canonical mnemonic; misses become
//! [`PpuDecodeError::EncodingNotRecognized`]. The decoder never
//! panics.
// [PPC-Book1 p:7 s:1.7 Instruction formats] OPCD at bits 0:5; XO is form-dependent.

use crate::instruction::known_encodings::{self, SprDirection};
use crate::instruction::{Locator, PpuDecodeError, PpuInstruction};

/// Construct the appropriate `PpuDecodeError` for a `(primary, xo)`
/// the decoder could not handle. Hits in [`known_encodings::opcode_gap`]
/// produce `DecoderArmUnimplemented` with the canonical mnemonic;
/// misses produce `EncodingNotRecognized`.
#[cold]
fn reject_opcode(raw: u32, primary: u8, xo: u16) -> PpuDecodeError {
    match known_encodings::opcode_gap(primary, xo) {
        Some(entry) => PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Opcode { primary, xo },
            mnemonic: entry.mnemonic,
            raw,
        },
        None => PpuDecodeError::EncodingNotRecognized { raw },
    }
}

/// Construct the appropriate `PpuDecodeError` for an unhandled SPR /
/// TBR under a known XFX opcode (`mfspr` / `mftb` / `mtspr`).
#[cold]
fn reject_spr(raw: u32, direction: SprDirection, spr: u16) -> PpuDecodeError {
    match known_encodings::spr_gap(direction, spr) {
        Some(entry) => PpuDecodeError::DecoderArmUnimplemented {
            locator: Locator::Spr {
                op_mnemonic: direction.op_mnemonic(),
                spr,
            },
            mnemonic: entry.mnemonic,
            raw,
        },
        None => PpuDecodeError::EncodingNotRecognized { raw },
    }
}

/// Extract D-form fields as `(rt/rs, ra, signed imm16)`.
// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) D/SI(16:31).
#[inline]
fn d_form(raw: u32) -> (u8, u8, i16) {
    (
        ((raw >> 21) & 0x1F) as u8,
        ((raw >> 16) & 0x1F) as u8,
        (raw & 0xFFFF) as i16,
    )
}

/// Extract D-form fields as `(rt/rs, ra, unsigned imm16)`.
// [PPC-Book1 p:8 s:1.7.4 D-Form] UI variant; immediate at bits 16:31, zero-extended.
#[inline]
fn d_form_u(raw: u32) -> (u8, u8, u16) {
    (
        ((raw >> 21) & 0x1F) as u8,
        ((raw >> 16) & 0x1F) as u8,
        (raw & 0xFFFF) as u16,
    )
}

/// Extract X-form fields as `(rt/rs, ra, rb)`.
// [PPC-Book1 p:9 s:1.7.6 X-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) RB(16:20) XO(21:30) Rc(31).
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
/// Returns [`PpuDecodeError::DecoderArmUnimplemented`] when the
/// encoding hits the spec-citation directory in
/// [`crate::instruction::known_encodings`] but has no decoder arm;
/// returns [`PpuDecodeError::EncodingNotRecognized`] when the
/// directory misses (garbage data, mis-aligned execution, or an
/// out-of-scope encoding).
pub fn decode(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    // [PPC-Book1 p:7 s:1.7 Instruction formats] Bits 0:5 always specify OPCD; shift 26 isolates them.
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
        13 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::AddicDot { rt, ra, imm })
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
        // [PPC-Book1 p:36 s:3.3] lhau D-form: load halfword algebraic with update.
        43 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lhau { rt, ra, imm })
        }
        // [PPC-Book1 p:54 s:3.3] lmw D-form: load multiple word, EA word-aligned.
        46 => {
            let (rt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lmw { rt, ra, imm })
        }
        // [PPC-Book1 p:54 s:3.3] stmw D-form: store multiple word, EA word-aligned.
        47 => {
            let (rs, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Stmw { rs, ra, imm })
        }
        58 => {
            // [PPC-Book1 p:8 s:1.7.5 DS-Form] DS(16:29) || 0b00, sign-extended; XO(30:31) selects op.
            // DS-form: low 2 bits select between ld/ldu/lwa.
            // `raw & 0xFFFC` masks off bits 0,1 so the resulting i16
            // has its low 2 bits zero by construction (DS-form is
            // word-aligned).
            let sub = raw & 0x3;
            let (rt, ra, _) = d_form(raw);
            let imm = (raw & 0xFFFC) as i16;
            match sub {
                0 => Ok(PpuInstruction::Ld { rt, ra, imm }),
                1 => Ok(PpuInstruction::Ldu { rt, ra, imm }),
                2 => Ok(PpuInstruction::Lwa { rt, ra, imm }),
                _ => Err(reject_opcode(raw, 58, sub as u16)),
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
            // [PPC-Book1 p:8 s:1.7.5 DS-Form] XO(30:31) selects std/stdu under primary 62.
            // DS-form: low 2 bits select between std/stdu. `raw & 0xFFFC`
            // masks off bits 0,1 so the resulting i16 has its low 2 bits
            // zero by construction (DS-form is word-aligned).
            let sub = raw & 0x3;
            let (rs, ra, _) = d_form(raw);
            let imm = (raw & 0xFFFC) as i16;
            match sub {
                0 => Ok(PpuInstruction::Std { rs, ra, imm }),
                1 => Ok(PpuInstruction::Stdu { rs, ra, imm }),
                _ => Err(reject_opcode(raw, 62, sub as u16)),
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
        29 => {
            let (rs, ra, imm) = d_form_u(raw);
            Ok(PpuInstruction::AndisDot { ra, rs, imm })
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
            // [PPC-Book1 p:8 s:1.7.1 I-Form] OPCD(0:5) LI(6:29) AA(30) LK(31); LI||0b00 sign-extended.
            // The 24-bit LI field at bits 6..29, concatenated with 0b00,
            // forms a 26-bit signed displacement. Mask off OPCD and the
            // AA/LK bits to isolate it, then sign-extend by shifting it
            // up to the MSB and arithmetic-shifting back. Arithmetic
            // semantics are guaranteed by Rust's signed integer right
            // shift on `i32`.
            let li = (raw & 0x03FF_FFFC) as i32;
            let offset = (li << 6) >> 6;
            let aa = raw & 2 != 0;
            let link = raw & 1 != 0;
            Ok(PpuInstruction::B { offset, aa, link })
        }

        16 => {
            // [PPC-Book1 p:8 s:1.7.2 B-Form] OPCD(0:5) BO(6:10) BI(11:15) BD(16:29) AA(30) LK(31).
            let bo = ((raw >> 21) & 0x1F) as u8;
            let bi = ((raw >> 16) & 0x1F) as u8;
            let bd = (raw & 0xFFFC) as i16;
            let aa = raw & 2 != 0;
            let link = raw & 1 != 0;
            Ok(PpuInstruction::Bc {
                bo,
                bi,
                offset: bd,
                aa,
                link,
            })
        }

        19 => decode_xl(raw),

        // [PPC-Book1 p:10 s:1.7.13 M-Form] OPCD RS RA RB/SH MB(21:25) ME(26:30) Rc.
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
        // [PPC-Book1 p:104 s:4.6.2] lfsu D-form: load single with update, single -> double in FRT.
        49 => {
            let (frt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lfsu { frt, ra, imm })
        }
        50 => {
            let (frt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lfd { frt, ra, imm })
        }
        // [PPC-Book1 p:105 s:4.6.2] lfdu D-form: load double with update.
        51 => {
            let (frt, ra, imm) = d_form(raw);
            Ok(PpuInstruction::Lfdu { frt, ra, imm })
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
            // [PPC-Book1 p:10 s:1.7.12 A-Form] OPCD FRT FRA FRB FRC(21:25) XO(26:30) Rc(31).
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
            // [PPC-Book1 p:8 s:1.7.3 SC-Form] OPCD ///(6:10) ///(11:15) //(16:19) LEV(20:26) //(27:29) 1(30) /(31).
            // [PPC-Book1 p:26 s:2.4.2 System Call Instruction] LEV occupies PPC bits 20..=26 (7 bits); LSB-0 that is bits 5..=11.
            // Bit 30 must be 1 (the `sc` form marker); bit 31 reserved.
            // Accepting any primary-17 word would let bit-30-clear
            // patterns silently route to LV2 syscall dispatch.
            if (raw & 0x3) != 0x2 {
                return Err(reject_opcode(raw, 17, 0));
            }
            let lev = ((raw >> 5) & 0x7F) as u8;
            Ok(PpuInstruction::Sc { lev })
        }

        // Top-level primary opcode unhandled. Use xo=0 as the lookup
        // key for primaries whose D / I / B forms have no extended
        // opcode; a hit names the instruction, a miss surfaces as
        // EncodingNotRecognized.
        _ => Err(reject_opcode(raw, primary as u8, 0)),
    }
}

/// Decode primary opcode 4: VA-form (XO bits 0..5 in 0x20..=0x2f) or
/// VX-form (XO bits 21..31, four register operands).
///
/// Unknown encodings reject via [`reject_opcode`] -- the catch-all
/// `Ok(Vx { xo })` / `Ok(Va { xo })` is gated by the
/// `KNOWN_VX_XOS` / `KNOWN_VA_XOS` directories so a primary-4 word
/// whose XO is not documented in AltiVec-PEM cannot silently
/// fabricate an opaque stub variant.
fn decode_vx(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let vt = ((raw >> 21) & 0x1F) as u8;
    let va = ((raw >> 16) & 0x1F) as u8;
    let vb = ((raw >> 11) & 0x1F) as u8;
    let vc = ((raw >> 6) & 0x1F) as u8;
    let xo_11 = (raw & 0x7FF) as u16;
    let xo_6 = (raw & 0x3F) as u8;

    if let 0x20..=0x2f = xo_6 {
        // [AltiVec-PEM p:A-21 s:A.5 Table A-5 VA-Form] OPCD vD vA vB vC(21:25) XO(26:31).
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
        if known_encodings::is_known_va(xo_6) {
            return Ok(PpuInstruction::Va {
                xo: xo_6,
                vt,
                va,
                vb,
                vc,
            });
        }
        return Err(reject_opcode(raw, 4, xo_6 as u16));
    }

    // [AltiVec-PEM p:A-21 s:A.5 Table A-6 VX-Form] OPCD vD vA vB XO(21:31).
    match xo_11 {
        0x4c4 => Ok(PpuInstruction::Vxor { vt, va, vb }),
        xo if known_encodings::is_known_vx(xo) => Ok(PpuInstruction::Vx { xo, vt, va, vb }),
        _ => Err(reject_opcode(raw, 4, xo_11)),
    }
}

/// Decode primary opcode 30 (MD-form: rldicl, rldicr, rldic, rldimi).
///
/// MD-form splits the 6-bit SH across bits 16..20 (low) and bit 30
/// (high); the 6-bit mask bound splits across bits 21..25 (low) and
/// bit 26 (high). Sub-opcode lives in bits 27..29.
// [PPC-Book1 p:10 s:1.7.14 MD-Form] OPCD RS RA sh(16:20) mb(21:25,26) XO(27:29) sh(30) Rc.
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
        _ => {
            // MDS-form rldcl (XO=8) / rldcr (XO=9) share primary
            // 30 but key on the 4-bit field at PPC bits 27..30
            // (LSB-0 bits 1..4), not the 3-bit MD-form XO. The
            // MDS RB register lives in the slot MD-form uses for
            // sh_lo (PPC bits 16..20).
            let mds_xo = ((raw >> 1) & 0xF) as u16;
            // [PPC-Book1 p:75 s:3.3.12] rldcl: clear left from mb; ROTL64(RS, RB[58:63]).
            // [PPC-Book1 p:75 s:3.3.12] rldcr: clear right of me; ROTL64(RS, RB[58:63]).
            let rb = sh_lo;
            match mds_xo {
                8 => {
                    return Ok(PpuInstruction::Rldcl {
                        ra,
                        rs,
                        rb,
                        mb: mask,
                        rc,
                    });
                }
                9 => {
                    return Ok(PpuInstruction::Rldcr {
                        ra,
                        rs,
                        rb,
                        me: mask,
                        rc,
                    });
                }
                _ => {}
            }
            // Unknown MDS XO: route through the directory; a miss
            // falls back to the (primary, md_xo) pair so the error
            // names what was actually decoded.
            if let Some(entry) = known_encodings::opcode_gap(30, mds_xo) {
                return Err(PpuDecodeError::DecoderArmUnimplemented {
                    locator: Locator::Opcode {
                        primary: 30,
                        xo: mds_xo,
                    },
                    mnemonic: entry.mnemonic,
                    raw,
                });
            }
            Err(reject_opcode(raw, 30, xo as u16))
        }
    }
}

/// Decode primary opcode 19 (XL-form: bclr, bcctr, isync, CR-logical).
// [PPC-Book1 p:9 s:1.7.7 XL-Form] OPCD BT/BO BA/BI BB(16:20) XO(21:30) LK(31).
fn decode_xl(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let xo = (raw >> 1) & 0x3FF;
    // Branch fields: bo at bits 6-10, bi at bits 11-15, lk at bit 31.
    // CR-logical's BT/BA share those positions; bb at bits 16-20.
    let bo = ((raw >> 21) & 0x1F) as u8;
    let bi = ((raw >> 16) & 0x1F) as u8;
    let link = raw & 1 != 0;
    let (bt, ba) = (bo, bi);
    let bb = ((raw >> 11) & 0x1F) as u8;
    // mcrf uses 3-bit CR-field selectors at bits 6-8 (BF) and 11-13 (BFA).
    let crfd = ((raw >> 23) & 0x7) as u8;
    let crfs = ((raw >> 18) & 0x7) as u8;

    match xo {
        0 => Ok(PpuInstruction::Mcrf { crfd, crfs }),
        16 => Ok(PpuInstruction::Bclr { bo, bi, link }),
        33 => Ok(PpuInstruction::Crnor { bt, ba, bb }),
        129 => Ok(PpuInstruction::Crandc { bt, ba, bb }),
        // isync decodes as `ori 0,0,0` (a nop under the deterministic model).
        150 => Ok(PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        }),
        193 => Ok(PpuInstruction::Crxor { bt, ba, bb }),
        225 => Ok(PpuInstruction::Crnand { bt, ba, bb }),
        257 => Ok(PpuInstruction::Crand { bt, ba, bb }),
        289 => Ok(PpuInstruction::Creqv { bt, ba, bb }),
        417 => Ok(PpuInstruction::Crorc { bt, ba, bb }),
        449 => Ok(PpuInstruction::Cror { bt, ba, bb }),
        528 => Ok(PpuInstruction::Bcctr { bo, bi, link }),
        _ => Err(reject_opcode(raw, 19, xo as u16)),
    }
}

/// Decode primary opcode 31 (X-form and XO-form).
///
/// XO-form uses a 9-bit extended opcode at bits 22..30; X-form uses
/// a 10-bit extended opcode at bits 21..30. The 9-bit match runs
/// first; on miss the 10-bit match takes over.
// [PPC-Book1 p:9 s:1.7.11 XO-Form] OPCD RT RA RB OE(21) XO(22:30) Rc(31).
// [PPC-Book1 p:208 s:Appendix I Opcode Maps] Primary 31 extended opcodes at bits 21:30.
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
        // [PPC-Book1 p:55 s:3.3.8] subfze (200), subfme (232), addme (234) XO-form arith.
        200 => return Ok(PpuInstruction::Subfze { rt, ra, oe, rc }),
        232 => return Ok(PpuInstruction::Subfme { rt, ra, oe, rc }),
        234 => return Ok(PpuInstruction::Addme { rt, ra, oe, rc }),
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
        // [PPC-Book1 p:65 s:3.3.13] eqv (284, XNOR) and nand (476).
        284 => return Ok(PpuInstruction::Eqv { ra, rs: rt, rb, rc }),
        476 => return Ok(PpuInstruction::Nand { ra, rs: rt, rb, rc }),

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
        // [PPC-Book1 p:9 s:1.7.10 XS-Form] sh(16:20) XO(21:29) sh(30) Rc(31).
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
        // [PPC-Book1 p:70 s:3.3.13] popcntb: per-byte 1-bit population count; no Rc.
        122 => return Ok(PpuInstruction::Popcntb { ra, rs: rt }),
        // [PPC-Book1 p:64 s:3.3.10] tw / td: TO field rides in the rt slot (bits 6..10).
        4 => return Ok(PpuInstruction::Tw { to: rt, ra, rb }),
        68 => return Ok(PpuInstruction::Td { to: rt, ra, rb }),
        // [PPC-Book1 p:135 s:6.1] mcrxr: BF occupies bits 6..8; bits 9..10 reserved.
        512 => return Ok(PpuInstruction::Mcrxr { bf: rt >> 2 }),

        922 => return Ok(PpuInstruction::Extsh { ra, rs: rt, rc }),
        954 => return Ok(PpuInstruction::Extsb { ra, rs: rt, rc }),
        986 => return Ok(PpuInstruction::Extsw { ra, rs: rt, rc }),

        23 => return Ok(PpuInstruction::Lwzx { rt, ra, rb }),
        87 => return Ok(PpuInstruction::Lbzx { rt, ra, rb }),
        21 => return Ok(PpuInstruction::Ldx { rt, ra, rb }),
        279 => return Ok(PpuInstruction::Lhzx { rt, ra, rb }),
        // [PPC-Book1 p:34 s:3.3.1] X-form indexed loads with update.
        55 => return Ok(PpuInstruction::Lwzux { rt, ra, rb }),
        119 => return Ok(PpuInstruction::Lbzux { rt, ra, rb }),
        311 => return Ok(PpuInstruction::Lhzux { rt, ra, rb }),
        53 => return Ok(PpuInstruction::Ldux { rt, ra, rb }),
        // [PPC-Book1 p:36 s:3.3] lhax / lhaux: sign-extend halfword; lwax / lwaux: sign-extend word.
        343 => return Ok(PpuInstruction::Lhax { rt, ra, rb }),
        375 => return Ok(PpuInstruction::Lhaux { rt, ra, rb }),
        341 => return Ok(PpuInstruction::Lwax { rt, ra, rb }),
        373 => return Ok(PpuInstruction::Lwaux { rt, ra, rb }),
        // [PPC-Book1 p:41 s:3.3.3] X-form indexed stores; update forms require RA != 0.
        407 => return Ok(PpuInstruction::Sthx { rs: rt, ra, rb }),
        439 => return Ok(PpuInstruction::Sthux { rs: rt, ra, rb }),
        183 => return Ok(PpuInstruction::Stwux { rs: rt, ra, rb }),
        247 => return Ok(PpuInstruction::Stbux { rs: rt, ra, rb }),

        // [PPC-Book1 p:55 s:3.3.5] Load / store string. Immediate-count
        // forms (lswi / stswi) store NB in the X-form RB slot; an NB
        // of 0 encodes a 32-byte transfer.
        597 => return Ok(PpuInstruction::Lswi { rt, ra, nb: rb }),
        725 => return Ok(PpuInstruction::Stswi { rs: rt, ra, nb: rb }),
        533 => return Ok(PpuInstruction::Lswx { rt, ra, rb }),
        661 => return Ok(PpuInstruction::Stswx { rs: rt, ra, rb }),

        // Cell BE PPU unaligned-vector loads and stores.
        // [CBE-Handbook p:744 s:A.3.3] PPE-only VMX misaligned helpers.
        519 => return Ok(PpuInstruction::Lvlx { vt: rt, ra, rb }),
        583 => return Ok(PpuInstruction::Lvrx { vt: rt, ra, rb }),
        647 => return Ok(PpuInstruction::Lvlxl { vt: rt, ra, rb }),
        711 => return Ok(PpuInstruction::Lvrxl { vt: rt, ra, rb }),
        775 => return Ok(PpuInstruction::Stvlx { vs: rt, ra, rb }),
        839 => return Ok(PpuInstruction::Stvrx { vs: rt, ra, rb }),
        903 => return Ok(PpuInstruction::Stvlxl { vs: rt, ra, rb }),
        967 => return Ok(PpuInstruction::Stvrxl { vs: rt, ra, rb }),

        // AltiVec-memory family (X-form, primary 31).
        // [AltiVec-PEM p:6-21 s:6.2] lvsl XO=6, [p:6-15] lvebx XO=7,
        // [p:6-22] lvsr XO=38, [p:6-16] lvehx XO=39, [p:6-17] lvewx XO=71,
        // [p:6-21] lvx XO=103, [p:6-29] stvebx XO=135, [p:6-30] stvehx XO=167,
        // [p:6-31] stvewx XO=199, [p:6-23] lvxl XO=359, [p:6-33] stvxl XO=487.
        6 => return Ok(PpuInstruction::Lvsl { vt: rt, ra, rb }),
        7 => return Ok(PpuInstruction::Lvebx { vt: rt, ra, rb }),
        38 => return Ok(PpuInstruction::Lvsr { vt: rt, ra, rb }),
        39 => return Ok(PpuInstruction::Lvehx { vt: rt, ra, rb }),
        71 => return Ok(PpuInstruction::Lvewx { vt: rt, ra, rb }),
        135 => return Ok(PpuInstruction::Stvebx { vs: rt, ra, rb }),
        167 => return Ok(PpuInstruction::Stvehx { vs: rt, ra, rb }),
        199 => return Ok(PpuInstruction::Stvewx { vs: rt, ra, rb }),
        359 => return Ok(PpuInstruction::Lvxl { vt: rt, ra, rb }),
        487 => return Ok(PpuInstruction::Stvxl { vs: rt, ra, rb }),

        84 => return Ok(PpuInstruction::Ldarx { rt, ra, rb }),
        // [PPC-Book2 p:25 s:3.3] stdcx. always has Rc=1 in the mnemonic; an
        // encoding with XO=214 and Rc=0 is a reserved form and must reject.
        214 if rc => return Ok(PpuInstruction::Stdcx { rs: rt, ra, rb }),
        20 => return Ok(PpuInstruction::Lwarx { rt, ra, rb }),
        // [PPC-Book2 p:25 s:3.3] stwcx. always has Rc=1; XO=150 with Rc=0
        // is a reserved encoding.
        150 if rc => return Ok(PpuInstruction::Stwcx { rs: rt, ra, rb }),

        151 => return Ok(PpuInstruction::Stwx { rs: rt, ra, rb }),
        149 => return Ok(PpuInstruction::Stdx { rs: rt, ra, rb }),
        181 => return Ok(PpuInstruction::Stdux { rs: rt, ra, rb }),
        215 => return Ok(PpuInstruction::Stbx { rs: rt, ra, rb }),

        // stfiwx reuses the RT slot for FRS.
        983 => return Ok(PpuInstruction::Stfiwx { frs: rt, ra, rb }),

        // X-form FP loads/stores. RT slot doubles as FRT/FRS.
        535 => return Ok(PpuInstruction::Lfsx { frt: rt, ra, rb }),
        567 => return Ok(PpuInstruction::Lfsux { frt: rt, ra, rb }),
        599 => return Ok(PpuInstruction::Lfdx { frt: rt, ra, rb }),
        631 => return Ok(PpuInstruction::Lfdux { frt: rt, ra, rb }),
        663 => return Ok(PpuInstruction::Stfsx { frs: rt, ra, rb }),
        695 => return Ok(PpuInstruction::Stfsux { frs: rt, ra, rb }),
        727 => return Ok(PpuInstruction::Stfdx { frs: rt, ra, rb }),
        759 => return Ok(PpuInstruction::Stfdux { frs: rt, ra, rb }),

        103 => return Ok(PpuInstruction::Lvx { vt: rt, ra, rb }),
        231 => return Ok(PpuInstruction::Stvx { vs: rt, ra, rb }),

        // Byte-reverse indexed loads and stores.
        // [PPC-Book1 p:50 s:3.3.4] lwbrx X-form (XO=534), lhbrx (XO=790).
        // [PPC-Book1 p:51 s:3.3.4] ldbrx X-form (XO=532), stwbrx (XO=662), sthbrx (XO=918).
        // [CBE-Handbook p:734 s:A.2.1] sdbrx (XO=660; CG-canonical spelling).
        532 => return Ok(PpuInstruction::Ldbrx { rt, ra, rb }),
        534 => return Ok(PpuInstruction::Lwbrx { rt, ra, rb }),
        660 => return Ok(PpuInstruction::Sdbrx { rs: rt, ra, rb }),
        662 => return Ok(PpuInstruction::Stwbrx { rs: rt, ra, rb }),
        790 => return Ok(PpuInstruction::Lhbrx { rt, ra, rb }),
        918 => return Ok(PpuInstruction::Sthbrx { rs: rt, ra, rb }),

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

        19 => {
            // PPC bit 11 (raw bit 20) distinguishes `mfcr` (=0,
            // copy all 8 CR fields) from `mfocrf` (=1, copy one
            // selected CR field). The latter is optional in
            // PowerPC and per CBE-Handbook p:738 s:A.2.3.1 it IS
            // implemented on the Cell PPE (cellSync2 and friends).
            // The CRM field at bits 12..19 selects the CR field for
            // the one-hot form; route to a typed `Mfocrf` variant so
            // the executor can apply the one-field-only semantic
            // distinct from `mfcr`.
            if (raw >> 20) & 1 == 1 {
                let crm = ((raw >> 12) & 0xFF) as u8;
                return Ok(PpuInstruction::Mfocrf { rt, crm });
            }
            return Ok(PpuInstruction::Mfcr { rt });
        }
        144 => {
            // [PPC-Book1 p:3 s:1.5.2 Reserved Fields and Reserved Values] reserved bits ignored on read.
            // mtcrf CRM (FXM field) lives at bits 12..19. PPC bit 11
            // (raw bit 20) distinguishes `mtcrf` (=0, full mask) from
            // `mtocrf` (=1, single-field). [CBE-Handbook p:738 s:A.2.3.1]
            // names mtocrf as a Book-I optional instruction the Cell
            // PPE implements (cellSync2 and friends); both forms
            // route to typed variants so neither silently masquerades
            // as the other.
            let crm = ((raw >> 12) & 0xFF) as u8;
            if (raw >> 20) & 1 == 1 {
                return Ok(PpuInstruction::Mtocrf { rs: rt, crm });
            }
            return Ok(PpuInstruction::Mtcrf { rs: rt, crm });
        }
        // [PPC-Book1 p:9 s:1.7.8 XFX-Form] OPCD RT/RS spr(11:20) XO(21:30) /(31).
        // SPR / TBR half-swap. The encoded register number is split
        // across the RA and RB slots, and the halves are reversed:
        //   rb (bits 11..=15) = SPR/TBR low 5 bits
        //   ra (bits 16..=20) = SPR/TBR high 5 bits
        // So `spr_raw = (rb << 5) | ra`. Do not "simplify" the
        // shift direction; the apparent reversal is the encoding.
        339 => {
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            return match spr_raw {
                1 => Ok(PpuInstruction::Mfxer { rt }),
                8 => Ok(PpuInstruction::Mflr { rt }),
                9 => Ok(PpuInstruction::Mfctr { rt }),
                // [PPC-Book2 p:30 s:4.2 Reading the Time Base] mfspr
                // RT, 268/269 is the equivalent of mftb / mftbu;
                // Power ISA defines them as alternate spellings of
                // the same TB read. Cell PPE supports both. Route
                // SPR 268/269 here so a producer that emits the
                // mfspr spelling decodes correctly rather than
                // surfacing as a spurious EncodingNotRecognized.
                268 => Ok(PpuInstruction::Mftb { rt }),
                269 => Ok(PpuInstruction::Mftbu { rt }),
                // [AltiVec-PEM p:48 s:2.3.2 VRSAVE Register] VRSAVE
                // is SPR 256, a 32-bit user-accessible register the
                // compiler uses to mark which VRs need save/restore
                // across function calls. Half-swap encoding for
                // SPR 256 = 0x100: rb = 8 (high5), ra = 0 (low5),
                // producing the observed raw word 0x7c0042a6 in
                // SSHD/WipEout EBOOTs.
                256 => Ok(PpuInstruction::Mfvrsave { rt }),
                _ => Err(reject_spr(raw, SprDirection::MfSpr, spr_raw)),
            };
        }
        371 => {
            let tbr = ((rb as u16) << 5) | (ra as u16);
            return match tbr {
                268 => Ok(PpuInstruction::Mftb { rt }),
                269 => Ok(PpuInstruction::Mftbu { rt }),
                _ => Err(reject_spr(raw, SprDirection::MfTb, tbr)),
            };
        }
        467 => {
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            return match spr_raw {
                1 => Ok(PpuInstruction::Mtxer { rs: rt }),
                8 => Ok(PpuInstruction::Mtlr { rs: rt }),
                9 => Ok(PpuInstruction::Mtctr { rs: rt }),
                // [AltiVec-PEM p:48 s:2.3.2 VRSAVE Register]
                // mtspr 256, rS writes the AltiVec save-mask SPR.
                // Observed raw word 0x7c0043a6 in SSHD/WipEout
                // EBOOTs is mtvrsave with rS=0.
                256 => Ok(PpuInstruction::Mtvrsave { rs: rt }),
                _ => Err(reject_spr(raw, SprDirection::MtSpr, spr_raw)),
            };
        }

        // [PPC-Book2 p:20 s:3.2 Cache Management Instructions] dcbz: 128-byte zero store to the block containing EA.
        // Unlike the cache hints below, dcbz has architecturally
        // visible effects and needs a real variant.
        1014 => return Ok(PpuInstruction::Dcbz { ra, rb }),

        // Cache and memory-barrier hints. Under the deterministic
        // single-unit model these all collapse to a nop:
        //   [PPC-Book2 p:24 s:3.2] dcbst (54), dcbf (86), dcbt (278),
        //                          dcbtst (246), icbi (982).
        //   [PPC-Book2 p:23 s:3.1] sync / lwsync / ptesync (598)
        //                          (L-field selects the flavor).
        //   [PPC-Book2 p:23 s:3.1] eieio (854).
        //   [AltiVec-PEM p:6-117] dst (342), dstst (374),
        //                         dss (822): AltiVec data-stream
        //                         touch hints; no cache model means
        //                         no architectural side-effect.
        54 | 86 | 246 | 278 | 342 | 374 | 598 | 822 | 854 | 982 => {
            return Ok(PpuInstruction::Ori {
                ra: 0,
                rs: 0,
                imm: 0,
            })
        }

        _ => {}
    }

    // No 9-bit XO arm and no 10-bit XO arm matched. Report against
    // the 10-bit XO -- the canonical encoding key for primary 31
    // X-form / XO-form per Appendix I of PPC-Book1.
    Err(reject_opcode(raw, 31, xo_10 as u16))
}

#[cfg(test)]
#[path = "tests/decode_tests.rs"]
mod tests;
