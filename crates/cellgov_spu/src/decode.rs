//! SPU instruction decoder.
//!
//! SPU instructions are fixed-width 32-bit, big-endian. Opcode width
//! varies by format:
//!
//! - RRR  (4-bit opcode):  bits `[0:3]`
//! - RR   (11-bit opcode): bits `[0:10]`
//! - RI7  (11-bit opcode): bits `[0:10]`
//! - RI10 (8-bit opcode):  bits `[0:7]`
//! - RI16 (9-bit opcode):  bits `[0:8]`
//! - RI18 (7-bit opcode):  bits `[0:6]`
//
// [SPU-ISA p:28 s:2.3 Instruction Formats] RR/RRR/RI7 opcode-field bit ranges.
// [SPU-ISA p:29 s:2.3 Instruction Formats] RI10/RI16/RI18 opcode-field bit ranges.

use crate::instruction::{SpuDecodeError, SpuInstruction};

/// Decode a 32-bit SPU instruction word.
///
/// # Errors
///
/// Returns [`SpuDecodeError::Unsupported`] for encodings not
/// implemented.
pub fn decode(raw: u32) -> Result<SpuInstruction, SpuDecodeError> {
    let op4 = (raw >> 28) & 0xF;
    let op7 = (raw >> 25) & 0x7F;
    let op8 = (raw >> 24) & 0xFF;
    let op9 = (raw >> 23) & 0x1FF;
    let op11 = (raw >> 21) & 0x7FF;

    let rt7 = (raw & 0x7F) as u8;
    let ra7 = ((raw >> 7) & 0x7F) as u8;
    let rb7 = ((raw >> 14) & 0x7F) as u8;

    // RRR: OP[0:3] RT[4:10] RB[11:17] RA[18:24] RC[25:31].
    // [SPU-ISA p:220 s:9 Shufb] RRR opcode 0xB; RC at bits [25:31].
    if op4 == 0xB {
        return Ok(SpuInstruction::Shufb {
            rt: ((raw >> 21) & 0x7F) as u8,
            ra: ((raw >> 7) & 0x7F) as u8,
            rb: ((raw >> 14) & 0x7F) as u8,
            rc: (raw & 0x7F) as u8,
        });
    }

    // RR / RI7 (11-bit opcode).
    match op11 {
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
        0x000 => {
            // [SPU-ISA p:238 s:10 Stop and Signal] opcode 0x000; signal in bits [18:31].
            return Ok(SpuInstruction::Stop {
                signal: (raw & 0x3FFF) as u16,
            });
        }
        0x1A8 => return Ok(SpuInstruction::Bi { ra: ra7 }),
        0x201 => return Ok(SpuInstruction::Nop),
        0x001 => return Ok(SpuInstruction::Lnop),
        0x002 => return Ok(SpuInstruction::Sync),
        0x3D8 => return Ok(SpuInstruction::Heq),
        0x1AC => return Ok(SpuInstruction::Hbr),
        0x1B0 | 0x1B1 => return Ok(SpuInstruction::Hbrp),
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

    // RI7: 7-bit immediate shares bit position with rb in RR format.
    let i7 = rb7;
    if op11 == 0x1FF {
        return Ok(SpuInstruction::Shlqbyi {
            rt: rt7,
            ra: ra7,
            imm: i7 & 0x1F,
        });
    }

    // RI10 (8-bit opcode, 10-bit immediate at [14:23]).
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

    // RI16 (9-bit opcode, 16-bit immediate at [7:22]).
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

    // RI18 (7-bit opcode, 18-bit immediate at [7:24]).
    if op7 == 0x21 {
        let imm = (raw >> 7) & 0x3FFFF;
        return Ok(SpuInstruction::Ila { rt: rt7, imm });
    }

    // hbrr: prefix 0001001 in bits [0:6], ROH in [7:8].
    if op7 == 0x09 {
        return Ok(SpuInstruction::Hbrr);
    }

    Err(SpuDecodeError::Unsupported(raw))
}

// [SPU-ISA p:171 s:6 Lqd] RI10 imm10 is sign-extended to i16 before address compute.
fn sign_extend_10(val: u16) -> i16 {
    if val & 0x200 != 0 {
        (val | 0xFC00) as i16
    } else {
        val as i16
    }
}

#[cfg(test)]
#[path = "tests/decode_tests.rs"]
mod tests;
