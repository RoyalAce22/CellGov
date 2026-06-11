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
// Several encoding tests below set bit-field slots to literal `0`
// via `(0u32 << N)` -- the explicit shift makes the field
// placement readable next to its sibling shifts, so the
// identity-op lint is silenced for the whole tests module rather
// than collapsing the documented zeros at each site.
#[allow(clippy::identity_op)]
mod tests {
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
    fn lvsr_decodes_to_named_variant_post_40e() {
        // lvsr v12, r0, r9 = 0x7d80484c (primary 31, XO 38). Stage
        // 40E graduated the AltiVec-memory family into the decoder;
        // the previous Phase-39-terminal "missing lvsr" reject is
        // now a successful decode.
        let raw = 0x7d80_484c;
        let inst = decode(raw).expect("lvsr must decode after 40E");
        assert_eq!(
            inst,
            PpuInstruction::Lvsr {
                vt: 12,
                ra: 0,
                rb: 9
            }
        );
    }

    #[test]
    fn altivec_memory_family_decodes_canonical_encodings() {
        // Each row: (raw, expected variant). The raw words use a
        // fixed RT/VT/VS=0, RA=1, RB=2 with the XO from the
        // AltiVec-PEM Ch. 6 instruction table.
        let p31 = 31u32 << 26;
        let regs = (0u32 << 21) | (1u32 << 16) | (2u32 << 11);
        let mk = |xo: u32| p31 | regs | (xo << 1);
        let cases: &[(u32, PpuInstruction)] = &[
            (
                mk(6),
                PpuInstruction::Lvsl {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(7),
                PpuInstruction::Lvebx {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(38),
                PpuInstruction::Lvsr {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(39),
                PpuInstruction::Lvehx {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(71),
                PpuInstruction::Lvewx {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(103),
                PpuInstruction::Lvx {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(135),
                PpuInstruction::Stvebx {
                    vs: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(167),
                PpuInstruction::Stvehx {
                    vs: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(199),
                PpuInstruction::Stvewx {
                    vs: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(359),
                PpuInstruction::Lvxl {
                    vt: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
            (
                mk(487),
                PpuInstruction::Stvxl {
                    vs: 0,
                    ra: 1,
                    rb: 2,
                },
            ),
        ];
        for &(raw, ref expected) in cases {
            let inst = decode(raw).unwrap_or_else(|e| {
                panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}")
            });
            assert_eq!(&inst, expected, "raw 0x{raw:08x}");
        }
    }

    #[test]
    fn indexed_update_family_decodes_canonical_encodings() {
        // X-form primary 31; RT/RS=3, RA=4, RB=5.
        let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
        let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
        let cases: &[(u32, PpuInstruction)] = &[
            (
                mk(55),
                PpuInstruction::Lwzux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(119),
                PpuInstruction::Lbzux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(311),
                PpuInstruction::Lhzux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(53),
                PpuInstruction::Ldux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(343),
                PpuInstruction::Lhax {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(375),
                PpuInstruction::Lhaux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(341),
                PpuInstruction::Lwax {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(373),
                PpuInstruction::Lwaux {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(407),
                PpuInstruction::Sthx {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(439),
                PpuInstruction::Sthux {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(183),
                PpuInstruction::Stwux {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(247),
                PpuInstruction::Stbux {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
        ];
        for &(raw, ref expected) in cases {
            let inst = decode(raw).unwrap_or_else(|e| {
                panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}")
            });
            assert_eq!(&inst, expected, "raw 0x{raw:08x}");
        }
    }

    #[test]
    fn load_store_string_family_decode_to_named_variants() {
        // X-form primary 31; for lswi/stswi the rb-slot encodes NB.
        let p31 = 31u32 << 26;
        let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
        let mk = |xo: u32| p31 | regs | (xo << 1);
        assert_eq!(
            decode(mk(533)).unwrap(),
            PpuInstruction::Lswx {
                rt: 3,
                ra: 4,
                rb: 5
            }
        );
        assert_eq!(
            decode(mk(597)).unwrap(),
            PpuInstruction::Lswi {
                rt: 3,
                ra: 4,
                nb: 5
            }
        );
        assert_eq!(
            decode(mk(661)).unwrap(),
            PpuInstruction::Stswx {
                rs: 3,
                ra: 4,
                rb: 5
            }
        );
        assert_eq!(
            decode(mk(725)).unwrap(),
            PpuInstruction::Stswi {
                rs: 3,
                ra: 4,
                nb: 5
            }
        );
    }

    #[test]
    fn primary31_x_form_residue_decode_to_named_variants() {
        // Stage 40C.9: tw / td / popcntb / mcrxr.
        // tw/td: TO rides in the rt slot; bit 31 reserved (= 0).
        // popcntb: standard X-form; no Rc.
        // mcrxr: BF occupies bits 6..8 (rt high 3 bits); bits 9..10 reserved.
        let p31 = 31u32 << 26;
        // tw 12, r4, r5
        let raw_tw = p31 | (12u32 << 21) | (4u32 << 16) | (5u32 << 11) | (4u32 << 1);
        assert_eq!(
            decode(raw_tw).unwrap(),
            PpuInstruction::Tw {
                to: 12,
                ra: 4,
                rb: 5
            }
        );
        // td 24, r6, r7
        let raw_td = p31 | (24u32 << 21) | (6u32 << 16) | (7u32 << 11) | (68u32 << 1);
        assert_eq!(
            decode(raw_td).unwrap(),
            PpuInstruction::Td {
                to: 24,
                ra: 6,
                rb: 7
            }
        );
        // popcntb r3, r4
        let raw_popcntb = p31 | (4u32 << 21) | (3u32 << 16) | (122u32 << 1);
        assert_eq!(
            decode(raw_popcntb).unwrap(),
            PpuInstruction::Popcntb { ra: 3, rs: 4 }
        );
        // mcrxr cr3: BF=3 occupies bits 6..8; rt slot value = 3 << 2 = 12.
        let raw_mcrxr = p31 | (12u32 << 21) | (512u32 << 1);
        assert_eq!(decode(raw_mcrxr).unwrap(), PpuInstruction::Mcrxr { bf: 3 });
    }

    #[test]
    fn xo_arith_and_logical_family_decode_to_named_variants() {
        // Stage 40C.7: XO-form arith (subfze/subfme/addme) and
        // 2-op logical (eqv/nand). XO-form uses 9-bit XO; logical
        // uses 10-bit XO. Test the bit-exact encoding shape.
        let p31 = 31u32 << 26;
        let regs_xo = (3u32 << 21) | (4u32 << 16);
        let mk_xo9 = |xo: u32| p31 | regs_xo | (xo << 1);
        assert_eq!(
            decode(mk_xo9(200)).unwrap(),
            PpuInstruction::Subfze {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            }
        );
        assert_eq!(
            decode(mk_xo9(232)).unwrap(),
            PpuInstruction::Subfme {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            }
        );
        assert_eq!(
            decode(mk_xo9(234)).unwrap(),
            PpuInstruction::Addme {
                rt: 3,
                ra: 4,
                oe: false,
                rc: false,
            }
        );

        let regs_x = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
        let mk_x10 = |xo: u32| p31 | regs_x | (xo << 1);
        assert_eq!(
            decode(mk_x10(284)).unwrap(),
            PpuInstruction::Eqv {
                ra: 4,
                rs: 3,
                rb: 5,
                rc: false,
            }
        );
        assert_eq!(
            decode(mk_x10(476)).unwrap(),
            PpuInstruction::Nand {
                ra: 4,
                rs: 3,
                rb: 5,
                rc: false,
            }
        );
    }

    #[test]
    fn cache_hint_family_collapses_to_nop() {
        // The cache-hint and data-stream-touch ops at primary 31 /
        // XOs 246 (dcbtst), 342 (dst), 374 (dstst), 822 (dss)
        // collapse to an Ori-nop under CellGov's deterministic
        // single-unit no-cache model. The collapse is by XO only;
        // operand bits are irrelevant. (`dst` / `dstst` / `dss` use
        // the AltiVec T(6) || STRM(7..8) || RA(11..15) || RB(16..20)
        // layout, not standard X-form `(rt, ra, rb)` -- but those
        // fields are ignored by the nop, and `dss` shares XO 822
        // with `dssall`, distinguished by the A bit also ignored.)
        // The XO bits in this fixture are the only thing the arm
        // discriminates on.
        let nop = PpuInstruction::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        };
        let p31 = 31u32 << 26;
        for xo in [246u32, 342, 374, 822] {
            let raw = p31 | (xo << 1);
            let inst = decode(raw)
                .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} xo={xo}: {e:?}"));
            assert_eq!(inst, nop, "xo {xo} did not collapse to nop");
        }
    }

    #[test]
    fn d_form_scalar_gaps_decode_to_named_variants() {
        // Each row: (primary, raw, expected). RT/RS/FRT/FRS=0,
        // RA=1, imm=4 (low bits free since none of these are
        // DS-form). Stage 40C.1 promoted these 5 ops out of the
        // OPCODE_GAPS top-level fall-through.
        let mk = |primary: u32| (primary << 26) | (0u32 << 21) | (1u32 << 16) | 4u32;
        let cases: &[(u32, PpuInstruction)] = &[
            (
                mk(43),
                PpuInstruction::Lhau {
                    rt: 0,
                    ra: 1,
                    imm: 4,
                },
            ),
            (
                mk(46),
                PpuInstruction::Lmw {
                    rt: 0,
                    ra: 1,
                    imm: 4,
                },
            ),
            (
                mk(47),
                PpuInstruction::Stmw {
                    rs: 0,
                    ra: 1,
                    imm: 4,
                },
            ),
            (
                mk(49),
                PpuInstruction::Lfsu {
                    frt: 0,
                    ra: 1,
                    imm: 4,
                },
            ),
            (
                mk(51),
                PpuInstruction::Lfdu {
                    frt: 0,
                    ra: 1,
                    imm: 4,
                },
            ),
        ];
        for &(raw, ref expected) in cases {
            let inst = decode(raw).unwrap_or_else(|e| {
                panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}")
            });
            assert_eq!(&inst, expected, "raw 0x{raw:08x}");
        }
    }

    #[test]
    fn cbe_unaligned_vxu_family_decodes_canonical_encodings() {
        // X-form primary 31; VT/VS=2, RA=3, RB=4.
        let regs = (2u32 << 21) | (3u32 << 16) | (4u32 << 11);
        let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
        let cases: &[(u32, PpuInstruction)] = &[
            (
                mk(647),
                PpuInstruction::Lvlxl {
                    vt: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
            (
                mk(711),
                PpuInstruction::Lvrxl {
                    vt: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
            (
                mk(775),
                PpuInstruction::Stvlx {
                    vs: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
            (
                mk(839),
                PpuInstruction::Stvrx {
                    vs: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
            (
                mk(903),
                PpuInstruction::Stvlxl {
                    vs: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
            (
                mk(967),
                PpuInstruction::Stvrxl {
                    vs: 2,
                    ra: 3,
                    rb: 4,
                },
            ),
        ];
        for &(raw, ref expected) in cases {
            let inst = decode(raw).unwrap_or_else(|e| {
                panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}")
            });
            assert_eq!(&inst, expected, "raw 0x{raw:08x}");
        }
    }

    #[test]
    fn byte_reverse_family_decodes_canonical_encodings() {
        // X-form primary 31; RT/RS=3, RA=4, RB=5.
        let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
        let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
        let cases: &[(u32, PpuInstruction)] = &[
            (
                mk(532),
                PpuInstruction::Ldbrx {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(534),
                PpuInstruction::Lwbrx {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(660),
                PpuInstruction::Sdbrx {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(662),
                PpuInstruction::Stwbrx {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(790),
                PpuInstruction::Lhbrx {
                    rt: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                mk(918),
                PpuInstruction::Sthbrx {
                    rs: 3,
                    ra: 4,
                    rb: 5,
                },
            ),
        ];
        for &(raw, ref expected) in cases {
            let inst = decode(raw).unwrap_or_else(|e| {
                panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}")
            });
            assert_eq!(&inst, expected, "raw 0x{raw:08x}");
        }
    }

    #[test]
    fn mds_form_rldcl_rldcr_decode_to_named_variants() {
        // MDS-form: primary 30, RS=5, RA=6, RB=7, mask_lo=2,
        // mask_hi=1 -> mb/me = 0x22 = 34, Rc=0.
        // XO 8 (rldcl) and XO 9 (rldcr) live in PPC bits 27..30
        // (LSB-0 bits 1..4). The MD-form sub-XO bits 2..4 hit
        // XO=4 for both, which is reserved -- the MDS XO is the
        // distinguishing key.
        let common = (30u32 << 26)         // primary
            | (5u32 << 21)                  // rs = 5
            | (6u32 << 16)                  // ra = 6
            | (7u32 << 11)                  // rb = 7
            | (2u32 << 6)                   // mask_lo = 2
            | (1u32 << 5); // mask_hi = 1 -> mb/me = 34

        let rldcl_raw = common | (8u32 << 1); // MDS XO=8
        let rldcr_raw = common | (9u32 << 1); // MDS XO=9

        assert_eq!(
            decode(rldcl_raw).unwrap(),
            PpuInstruction::Rldcl {
                ra: 6,
                rs: 5,
                rb: 7,
                mb: 34,
                rc: false,
            }
        );
        assert_eq!(
            decode(rldcr_raw).unwrap(),
            PpuInstruction::Rldcr {
                ra: 6,
                rs: 5,
                rb: 7,
                me: 34,
                rc: false,
            }
        );
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

    #[test]
    fn mfspr_at_spr_1_decodes_to_mfxer() {
        // SPR=1 reads XER. Canonical encoding: ra=1, rb=0 (low half
        // ahead of high half per the XFX split).
        let raw: u32 = (31u32 << 26) | (5u32 << 21) | (1u32 << 16) | (339u32 << 1);
        assert_eq!(decode(raw).unwrap(), PpuInstruction::Mfxer { rt: 5 });
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
                other => panic!(
                    "primary-0 word {bits:#010x} must be EncodingNotRecognized, got {other:?}"
                ),
            }
        }
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
    fn cr_logical_family_decodes() {
        // BT=8, BA=9, BB=10 across each XO. The 5-bit fields lie at
        // raw bits (21..26), (16..21), (11..16) respectively.
        let mk = |xo: u32| (19u32 << 26) | (8u32 << 21) | (9u32 << 16) | (10u32 << 11) | (xo << 1);

        let cases: &[(u32, PpuInstruction)] = &[
            (
                33,
                PpuInstruction::Crnor {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                129,
                PpuInstruction::Crandc {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                193,
                PpuInstruction::Crxor {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                225,
                PpuInstruction::Crnand {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                257,
                PpuInstruction::Crand {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                289,
                PpuInstruction::Creqv {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                417,
                PpuInstruction::Crorc {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
            (
                449,
                PpuInstruction::Cror {
                    bt: 8,
                    ba: 9,
                    bb: 10,
                },
            ),
        ];
        for (xo, expected) in cases {
            let raw = mk(*xo);
            assert_eq!(decode(raw).unwrap(), *expected, "xo={xo}");
        }
    }

    #[test]
    fn mcrf_decodes() {
        // mcrf 5, 2: BF=5 at bits 6..9, BFA=2 at bits 11..14, XO=0.
        let raw = (19u32 << 26) | (5u32 << 23) | (2u32 << 18);
        let insn = decode(raw).unwrap();
        assert_eq!(insn, PpuInstruction::Mcrf { crfd: 5, crfs: 2 });
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
    fn lwa_decodes_at_primary_58_sub_2() {
        // lwa r3, 8(r4): primary=58, RT=3, RA=4, DS=2 (byte offset 8),
        // sub=2. Word: (58<<26) | (3<<21) | (4<<16) | 0x0008 | 2.
        let raw = (58u32 << 26) | (3u32 << 21) | (4u32 << 16) | 0x000A;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Lwa {
                rt: 3,
                ra: 4,
                imm: 8,
            }
        );
    }

    #[test]
    fn addic_dot_decodes_at_primary_13() {
        // addic. r3, r4, -1: primary=13, RT=3, RA=4, SIMM=0xFFFF.
        let raw = (13u32 << 26) | (3u32 << 21) | (4u32 << 16) | 0xFFFF;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::AddicDot {
                rt: 3,
                ra: 4,
                imm: -1,
            }
        );
    }

    #[test]
    fn andis_dot_decodes_at_primary_29() {
        // andis. r3, r4, 0x00FF: primary=29, RA=3, RS=4, UI=0xFF.
        let raw = (29u32 << 26) | (4u32 << 21) | (3u32 << 16) | 0x00FF;
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::AndisDot {
                ra: 3,
                rs: 4,
                imm: 0x00FF,
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
    fn lfsx_decodes() {
        // lfsx fr13, r3, r0.
        // Encoding: OP=31 | FRT=13 | RA=3 | RB=0 | XO(10)=535 | 0
        let insn = decode(0x7DA3_042E).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Lfsx {
                frt: 13,
                ra: 3,
                rb: 0,
            }
        );
    }

    #[test]
    fn x_form_fp_load_store_family_decodes() {
        // FRT/FRS=11, RA=4, RB=5 across each XO. The 5-bit fields lie
        // at raw bits (21..26), (16..21), (11..16) respectively;
        // X-form XO at bits 21..30 puts XO << 1 in the low half.
        let mk = |xo: u32| (31u32 << 26) | (11u32 << 21) | (4u32 << 16) | (5u32 << 11) | (xo << 1);

        let cases: &[(u32, PpuInstruction)] = &[
            (
                535,
                PpuInstruction::Lfsx {
                    frt: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                567,
                PpuInstruction::Lfsux {
                    frt: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                599,
                PpuInstruction::Lfdx {
                    frt: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                631,
                PpuInstruction::Lfdux {
                    frt: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                663,
                PpuInstruction::Stfsx {
                    frs: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                695,
                PpuInstruction::Stfsux {
                    frs: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                727,
                PpuInstruction::Stfdx {
                    frs: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
            (
                759,
                PpuInstruction::Stfdux {
                    frs: 11,
                    ra: 4,
                    rb: 5,
                },
            ),
        ];
        for (xo, expected) in cases {
            let raw = mk(*xo);
            assert_eq!(decode(raw).unwrap(), *expected, "xo={xo}");
        }
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
    fn dcbz_decodes_with_real_variant() {
        // dcbz r6, r7 -> primary 31, RA=6, RB=7, XO=1014.
        let raw: u32 = (31u32 << 26) | (6u32 << 16) | (7u32 << 11) | (1014u32 << 1);
        let insn = decode(raw).unwrap();
        assert_eq!(insn, PpuInstruction::Dcbz { ra: 6, rb: 7 });
    }

    #[test]
    fn cache_hints_still_collapse_to_nop() {
        // icbi (XO=982) remains nopped.
        let raw: u32 = (31u32 << 26) | (982u32 << 1);
        let insn = decode(raw).unwrap();
        assert_eq!(
            insn,
            PpuInstruction::Ori {
                ra: 0,
                rs: 0,
                imm: 0,
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

        let raw_269: u32 =
            (31u32 << 26) | (3u32 << 21) | (13u32 << 16) | (8u32 << 11) | (339u32 << 1);
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
    fn sc_with_bit30_set_decodes_with_lev() {
        // Bit 30 = 1, LEV = 0 -> standard kernel syscall.
        let raw: u32 = (17u32 << 26) | 0x2;
        match decode(raw).unwrap() {
            PpuInstruction::Sc { lev: 0 } => {}
            other => panic!("expected Sc lev=0, got {other:?}"),
        }
        // LEV = 1 -> hypercall form.
        let raw_hv: u32 = (17u32 << 26) | (1u32 << 5) | 0x2;
        match decode(raw_hv).unwrap() {
            PpuInstruction::Sc { lev: 1 } => {}
            other => panic!("expected Sc lev=1, got {other:?}"),
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
        let raw: u32 =
            (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (150u32 << 1) | 1;
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
        let raw: u32 =
            (31u32 << 26) | (1u32 << 21) | (2u32 << 16) | (3u32 << 11) | (214u32 << 1) | 1;
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
}
