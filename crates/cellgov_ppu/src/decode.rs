//! PPU instruction decoder.
//!
//! Pure function: 32-bit raw word in, typed `PpuInstruction` out.
//! No state, no Effects, no runtime knowledge. Field extraction only.
//!
//! PPC instructions are fixed-width 32-bit, big-endian. The primary
//! opcode occupies bits 0-5. Many instructions use an extended opcode
//! in bits 21-30 (XO-form) or other positions.

use crate::instruction::{PpuDecodeError, PpuInstruction};

/// Decode a 32-bit PPC instruction word.
///
/// Returns `Err(PpuDecodeError::Unsupported(raw))` for any encoding
/// not yet implemented.
pub fn decode(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let primary = (raw >> 26) & 0x3F;

    match primary {
        // VX-form: AltiVec / VMX (subset)
        4 => decode_vx(raw),

        // D-form: loads and stores
        32 => {
            // lwz
            let rt = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Lwz { rt, ra, imm })
        }
        34 => {
            // lbz
            let rt = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Lbz { rt, ra, imm })
        }
        58 => {
            // ld (DS-form): low 2 bits are the sub-opcode (0 = ld)
            let sub = raw & 0x3;
            if sub != 0 {
                return Err(PpuDecodeError::Unsupported(raw));
            }
            let rt = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFC) as i16;
            Ok(PpuInstruction::Ld { rt, ra, imm })
        }
        36 => {
            // stw
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Stw { rs, ra, imm })
        }
        37 => {
            // stwu
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Stwu { rs, ra, imm })
        }
        38 => {
            // stb
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Stb { rs, ra, imm })
        }
        44 => {
            // sth
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Sth { rs, ra, imm })
        }
        62 => {
            // std / stdu (DS-form): low 2 bits are sub-opcode
            let sub = raw & 0x3;
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFC) as i16;
            match sub {
                0 => Ok(PpuInstruction::Std { rs, ra, imm }),
                1 => Ok(PpuInstruction::Stdu { rs, ra, imm }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            }
        }

        // D-form: arithmetic / logical immediate
        14 => {
            // addi (li when ra=0)
            let rt = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Addi { rt, ra, imm })
        }
        15 => {
            // addis (lis when ra=0)
            let rt = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Addis { rt, ra, imm })
        }
        24 => {
            // ori (mr when imm=0 and rs=ra... but we represent it as ori)
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as u16;
            Ok(PpuInstruction::Ori { ra, rs, imm })
        }
        25 => {
            // oris
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as u16;
            Ok(PpuInstruction::Oris { ra, rs, imm })
        }

        // D-form: compare
        11 => {
            // cmpwi (L=0 for 32-bit compare)
            let bf = ((raw >> 23) & 0x7) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as i16;
            Ok(PpuInstruction::Cmpwi { bf, ra, imm })
        }
        10 => {
            // cmplwi (L=0 for 32-bit compare logical)
            let bf = ((raw >> 23) & 0x7) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let imm = (raw & 0xFFFF) as u16;
            Ok(PpuInstruction::Cmplwi { bf, ra, imm })
        }

        // I-form: unconditional branch
        18 => {
            let li = raw & 0x03FF_FFFC;
            // Sign-extend 26-bit offset
            let offset = if li & 0x0200_0000 != 0 {
                (li | 0xFC00_0000) as i32
            } else {
                li as i32
            };
            let link = raw & 1 != 0;
            Ok(PpuInstruction::B { offset, link })
        }

        // B-form: conditional branch
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

        // XL-form: bclr, bcctr, and other CR ops
        19 => decode_xl(raw),

        // Rotate/shift
        21 => {
            // rlwinm
            let rs = ((raw >> 21) & 0x1F) as u8;
            let ra = ((raw >> 16) & 0x1F) as u8;
            let sh = ((raw >> 11) & 0x1F) as u8;
            let mb = ((raw >> 6) & 0x1F) as u8;
            let me = ((raw >> 1) & 0x1F) as u8;
            Ok(PpuInstruction::Rlwinm { ra, rs, sh, mb, me })
        }
        30 => decode_md(raw),

        // XO-form: extended arithmetic (add, etc.)
        31 => decode_x31(raw),

        // System call
        17 => Ok(PpuInstruction::Sc),

        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 4 (VX-form: AltiVec / VMX).
///
/// VX-form carries an 11-bit extended opcode at bits 21-31. Only the
/// encodings actually produced by the microtest toolchain are
/// recognized; everything else yields `Unsupported`.
fn decode_vx(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let vt = ((raw >> 21) & 0x1F) as u8;
    let va = ((raw >> 16) & 0x1F) as u8;
    let vb = ((raw >> 11) & 0x1F) as u8;
    let xo = raw & 0x7FF; // 11-bit XO at bits 21..31

    match xo {
        1220 => Ok(PpuInstruction::Vxor { vt, va, vb }),
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 30 (MD-form: rldicl, rldicr, rldic, rldimi).
///
/// Fields (Power ISA bit positions, bit 0 = MSB of the 32-bit word):
/// - bits 6..10:  rs
/// - bits 11..15: ra
/// - bits 16..20: sh[0..4]  (low 5 bits of shift)
/// - bits 21..25: mb[0..4] / me[0..4] (low 5 bits of mask bound)
/// - bit 26:      mb[5] / me[5] (high-order bit of mask bound)
/// - bits 27..29: xo (0 = rldicl, 1 = rldicr)
/// - bit 30:      sh[5] (high-order bit of shift)
/// - bit 31:      Rc
fn decode_md(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let rs = ((raw >> 21) & 0x1F) as u8;
    let ra = ((raw >> 16) & 0x1F) as u8;
    let sh_lo = ((raw >> 11) & 0x1F) as u8;
    let mask_lo = ((raw >> 6) & 0x1F) as u8; // bits 21..25
    let mask_hi = ((raw >> 5) & 0x1) as u8; // bit 26
    let xo = ((raw >> 2) & 0x7) as u8;
    let sh_hi = ((raw >> 1) & 0x1) as u8; // bit 30
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
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 19 (XL-form: bclr, bcctr, etc.).
fn decode_xl(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let xo = (raw >> 1) & 0x3FF;
    let bo = ((raw >> 21) & 0x1F) as u8;
    let bi = ((raw >> 16) & 0x1F) as u8;
    let link = raw & 1 != 0;

    match xo {
        16 => Ok(PpuInstruction::Bclr { bo, bi, link }),
        528 => Ok(PpuInstruction::Bcctr { bo, bi, link }),
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

/// Decode primary opcode 31 (X/XO-form: add, mfspr, mtspr, etc.).
fn decode_x31(raw: u32) -> Result<PpuInstruction, PpuDecodeError> {
    let xo_10 = (raw >> 1) & 0x3FF; // 10-bit XO for X-form
    let xo_9 = (raw >> 1) & 0x1FF; // 9-bit XO for XO-form

    let rt = ((raw >> 21) & 0x1F) as u8;
    let ra = ((raw >> 16) & 0x1F) as u8;
    let rb = ((raw >> 11) & 0x1F) as u8;

    // XO-form: add (xo = 266)
    if xo_9 == 266 {
        return Ok(PpuInstruction::Add { rt, ra, rb });
    }

    // X-form: or (xo = 444). In X-form the source register is in the
    // same field position as rt (bits 21..26); the destination is ra.
    if xo_10 == 444 {
        return Ok(PpuInstruction::Or { ra, rs: rt, rb });
    }

    // X-form: stvx (xo = 231). The source vector register lives in
    // the same encoding slot as rt.
    if xo_10 == 231 {
        return Ok(PpuInstruction::Stvx { vs: rt, ra, rb });
    }

    // X-form: extsw (xo = 986). rs is the source, ra the destination.
    if xo_10 == 986 {
        return Ok(PpuInstruction::Extsw { ra, rs: rt });
    }

    // X-form: mfspr / mtspr
    match xo_10 {
        339 => {
            // mfspr: SPR is encoded as (spr[5:9] << 5) | spr[0:4]
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            match spr_raw {
                8 => Ok(PpuInstruction::Mflr { rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            }
        }
        467 => {
            // mtspr
            let spr_raw = ((rb as u16) << 5) | (ra as u16);
            match spr_raw {
                8 => Ok(PpuInstruction::Mtlr { rs: rt }),
                9 => Ok(PpuInstruction::Mtctr { rs: rt }),
                _ => Err(PpuDecodeError::Unsupported(raw)),
            }
        }
        _ => Err(PpuDecodeError::Unsupported(raw)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_returns_error() {
        let result = decode(0xFFFF_FFFF);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PpuDecodeError::Unsupported(0xFFFF_FFFF)
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
}
