//! SPU instruction decoder.
//!
//! Pure function: 32-bit raw word in, typed `SpuInstruction` out.
//! No state, no Effects, no runtime knowledge. Field extraction only.
//!
//! SPU instructions are fixed-width 32-bit, big-endian. The opcode
//! occupies the most significant bits, with format-dependent lengths:
//!
//! - RRR  (4-bit opcode):  bits `[0:3]`
//! - RR   (11-bit opcode): bits `[0:10]`
//! - RI7  (11-bit opcode): bits `[0:10]`
//! - RI10 (8-bit opcode):  bits `[0:7]`
//! - RI16 (9-bit opcode):  bits `[0:8]`
//! - RI18 (7-bit opcode):  bits `[0:6]`

use crate::instruction::{SpuDecodeError, SpuInstruction};

/// Decode a 32-bit SPU instruction word.
///
/// Returns `Err(SpuDecodeError::Unsupported(raw))` for any encoding
/// not yet implemented.
pub fn decode(raw: u32) -> Result<SpuInstruction, SpuDecodeError> {
    let op4 = (raw >> 28) & 0xF;
    let op7 = (raw >> 25) & 0x7F;
    let op8 = (raw >> 24) & 0xFF;
    let op9 = (raw >> 23) & 0x1FF;
    let op11 = (raw >> 21) & 0x7FF;

    // Extract common fields
    let rt7 = (raw & 0x7F) as u8;
    let ra7 = ((raw >> 7) & 0x7F) as u8;
    let rb7 = ((raw >> 14) & 0x7F) as u8;

    // RRR format (4-bit opcode) -- shufb
    if op4 == 0xB {
        // RRR format: OP[0:3] RT[4:10] RB[11:17] RA[18:24] RC[25:31]
        return Ok(SpuInstruction::Shufb {
            rt: ((raw >> 21) & 0x7F) as u8,
            ra: ((raw >> 7) & 0x7F) as u8,
            rb: ((raw >> 14) & 0x7F) as u8,
            rc: (raw & 0x7F) as u8,
        });
    }

    // RR format / RI7 format (11-bit opcode)
    match op11 {
        // Channel operations
        0x00D => {
            return Ok(SpuInstruction::Rdch {
                rt: rt7,
                channel: ra7,
            })
        }
        0x10D => {
            return Ok(SpuInstruction::Wrch {
                channel: ra7,
                rt: rt7,
            })
        }
        // Stop
        0x000 => {
            return Ok(SpuInstruction::Stop {
                signal: (raw & 0x3FFF) as u16,
            })
        }
        // Branch indirect
        0x1A8 => return Ok(SpuInstruction::Bi { ra: ra7 }),
        // Nop / lnop / sync / heq
        0x201 => return Ok(SpuInstruction::Nop),
        0x001 => return Ok(SpuInstruction::Lnop),
        0x002 => return Ok(SpuInstruction::Sync),
        0x3D8 => return Ok(SpuInstruction::Heq),
        // Hint for branch
        0x1AC => return Ok(SpuInstruction::Hbr),
        0x1B0 | 0x1B1 => return Ok(SpuInstruction::Hbrp),
        // RR arithmetic/logical
        0x0C0 => {
            return Ok(SpuInstruction::A {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x040 => {
            return Ok(SpuInstruction::Sf {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x049 => {
            return Ok(SpuInstruction::Nor {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x3C0 => {
            return Ok(SpuInstruction::Ceq {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x1DC => {
            return Ok(SpuInstruction::Rotqby {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // Generate controls
        0x1F4 => {
            return Ok(SpuInstruction::Cbd {
                rt: rt7,
                ra: ra7,
                imm: rb7,
            })
        }
        0x1F6 => {
            return Ok(SpuInstruction::Cwd {
                rt: rt7,
                ra: ra7,
                imm: rb7,
            })
        }
        _ => {}
    }

    // RI7 format (11-bit opcode, 7-bit immediate in bits [11:17])
    let i7 = rb7; // same bit position as rb in RR format
    if op11 == 0x1FF {
        // shlqbyi: shift left quadword by bytes immediate
        return Ok(SpuInstruction::Shlqbyi {
            rt: rt7,
            ra: ra7,
            imm: i7 & 0x1F,
        });
    }

    // RI10 format (8-bit opcode)
    let i10 = ((raw >> 14) & 0x3FF) as u16;
    match op8 {
        0x34 => {
            return Ok(SpuInstruction::Lqd {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x24 => {
            return Ok(SpuInstruction::Stqd {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x14 => {
            return Ok(SpuInstruction::Andi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x1C => {
            return Ok(SpuInstruction::Ai {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x04 => {
            return Ok(SpuInstruction::Ori {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x7C => {
            return Ok(SpuInstruction::Ceqi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        0x38 => {
            return Ok(SpuInstruction::Lqx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x28 => {
            return Ok(SpuInstruction::Stqx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        _ => {}
    }

    // RI16 format (9-bit opcode)
    let i16_raw = ((raw >> 7) & 0xFFFF) as u16;
    let i16_signed = i16_raw as i16;
    let i16_offset = i16_signed as i32;
    match op9 {
        0x081 => {
            return Ok(SpuInstruction::Il {
                rt: rt7,
                imm: i16_signed,
            })
        }
        0x082 => {
            return Ok(SpuInstruction::Ilhu {
                rt: rt7,
                imm: i16_raw,
            })
        }
        0x083 => {
            return Ok(SpuInstruction::Ilh {
                rt: rt7,
                imm: i16_raw,
            })
        }
        0x0C1 => {
            return Ok(SpuInstruction::Iohl {
                rt: rt7,
                imm: i16_raw,
            })
        }
        0x064 => return Ok(SpuInstruction::Br { offset: i16_offset }),
        0x066 => {
            return Ok(SpuInstruction::Brsl {
                rt: rt7,
                offset: i16_offset,
            })
        }
        0x040 => {
            return Ok(SpuInstruction::Brz {
                rt: rt7,
                offset: i16_offset,
            })
        }
        0x042 => {
            return Ok(SpuInstruction::Brnz {
                rt: rt7,
                offset: i16_offset,
            })
        }
        0x061 => {
            return Ok(SpuInstruction::Lqa {
                rt: rt7,
                imm: i16_signed,
            })
        }
        0x041 => {
            return Ok(SpuInstruction::Stqa {
                rt: rt7,
                imm: i16_signed,
            })
        }
        0x065 => {
            return Ok(SpuInstruction::Fsmbi {
                rt: rt7,
                imm: i16_raw,
            })
        }
        _ => {}
    }

    // RI18 format (7-bit opcode)
    if op7 == 0x21 {
        let imm = (raw >> 7) & 0x3FFFF;
        return Ok(SpuInstruction::Ila { rt: rt7, imm });
    }

    // hbrr: 7-bit prefix 0001001 (bits [0:6]), ROH in [7:8]
    if op7 == 0x09 {
        return Ok(SpuInstruction::Hbrr);
    }

    Err(SpuDecodeError::Unsupported(raw))
}

/// Sign-extend a 10-bit value to i16.
fn sign_extend_10(val: u16) -> i16 {
    if val & 0x200 != 0 {
        (val | 0xFC00) as i16
    } else {
        val as i16
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
            SpuDecodeError::Unsupported(0xFFFF_FFFF)
        );
    }

    #[test]
    fn stop_zero_decodes() {
        let insn = decode(0x0000_0000).unwrap();
        assert_eq!(insn, SpuInstruction::Stop { signal: 0 });
    }

    #[test]
    fn il_from_binary() {
        // il $3, 12288 -> 0x40980003 (from spu_fixed_value disasm)
        let insn = decode(0x4098_0003).unwrap();
        assert_eq!(insn, SpuInstruction::Il { rt: 3, imm: 12288 });
    }

    #[test]
    fn stqd_from_binary() {
        // stqd $0, 16($1) -> 0x24004080
        let insn = decode(0x2400_4080).unwrap();
        assert_eq!(
            insn,
            SpuInstruction::Stqd {
                rt: 0,
                ra: 1,
                imm: 1
            }
        );
    }

    #[test]
    fn ai_from_binary() {
        // ai $1, $1, -32 -> 0x1cf80081
        let insn = decode(0x1cf8_0081).unwrap();
        assert_eq!(
            insn,
            SpuInstruction::Ai {
                rt: 1,
                ra: 1,
                imm: -32
            }
        );
    }

    #[test]
    fn ilhu_from_binary() {
        // ilhu $16, 4919 -> 0x41099b90
        let insn = decode(0x4109_9b90).unwrap();
        assert_eq!(insn, SpuInstruction::Ilhu { rt: 16, imm: 4919 });
    }

    #[test]
    fn iohl_from_binary() {
        // iohl $16, 47789 -> 0x60dd5690
        let insn = decode(0x60dd_5690).unwrap();
        assert_eq!(insn, SpuInstruction::Iohl { rt: 16, imm: 47789 });
    }

    #[test]
    fn wrch_from_binary() {
        // wrch $ch16, $9 -> 0x21a00809
        let insn = decode(0x21a0_0809).unwrap();
        assert_eq!(insn, SpuInstruction::Wrch { channel: 16, rt: 9 });
    }

    #[test]
    fn rdch_from_binary() {
        // rdch $2, $ch24 -> 0x01a00c02
        let insn = decode(0x01a0_0c02).unwrap();
        assert_eq!(insn, SpuInstruction::Rdch { rt: 2, channel: 24 });
    }

    #[test]
    fn ori_from_binary() {
        // ori $9, $3, 0 -> 0x04000189
        let insn = decode(0x0400_0189).unwrap();
        assert_eq!(
            insn,
            SpuInstruction::Ori {
                rt: 9,
                ra: 3,
                imm: 0
            }
        );
    }

    #[test]
    fn bi_from_binary() {
        // bi $0 -> 0x35000000
        let insn = decode(0x3500_0000).unwrap();
        assert_eq!(insn, SpuInstruction::Bi { ra: 0 });
    }

    #[test]
    fn brsl_from_binary() {
        // brsl $0, 0x378 -> 0x33006d00 (offset from PC)
        let insn = decode(0x3300_6d00).unwrap();
        assert!(matches!(insn, SpuInstruction::Brsl { rt: 0, .. }));
    }

    #[test]
    fn nop_from_binary() {
        // nop $127 -> 0x4020007f
        let insn = decode(0x4020_007f).unwrap();
        assert_eq!(insn, SpuInstruction::Nop);
    }

    #[test]
    fn lnop_from_binary() {
        // lnop -> 0x00200000
        let insn = decode(0x0020_0000).unwrap();
        assert_eq!(insn, SpuInstruction::Lnop);
    }

    #[test]
    fn shufb_from_binary() {
        // shufb $11, $2, $8, $5 -> 0xb1620105
        let insn = decode(0xb162_0105).unwrap();
        assert_eq!(
            insn,
            SpuInstruction::Shufb {
                rt: 11,
                ra: 2,
                rb: 8,
                rc: 5
            }
        );
    }

    #[test]
    fn cwd_from_binary() {
        // cwd $5, 0($3) -> 0x3ec00185
        let insn = decode(0x3ec0_0185).unwrap();
        assert_eq!(
            insn,
            SpuInstruction::Cwd {
                rt: 5,
                ra: 3,
                imm: 0
            }
        );
    }
}
