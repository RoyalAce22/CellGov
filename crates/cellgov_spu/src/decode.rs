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
/// Returns [`SpuDecodeError::Unassigned`] for a word that is not an SPU
/// instruction and [`SpuDecodeError::Unimplemented`] for an instruction
/// CellGov does not implement.
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
    // [SPU-ISA p:76 s:5 Mpya] RRR opcode 0xC.
    if op4 == 0xC {
        return Ok(SpuInstruction::Mpya {
            rt: ((raw >> 21) & 0x7F) as u8,
            ra: ((raw >> 7) & 0x7F) as u8,
            rb: ((raw >> 14) & 0x7F) as u8,
            rc: (raw & 0x7F) as u8,
        });
    }
    // [SPU-ISA p:115 s:5 Selb] RRR opcode 0x8.
    if op4 == 0x8 {
        return Ok(SpuInstruction::Selb {
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
        // [SPU-ISA p:239 s:10 Stopd] RR opcode 0x140; its RB, RA and RC fields only carry dependencies.
        0x140 => return Ok(SpuInstruction::Stopd),
        // [SPU-ISA p:244 s:10 Mfspr] RR opcode 0x00C; SA sits in the RA field.
        0x00C => return Ok(SpuInstruction::Mfspr { rt: rt7, sa: ra7 }),
        // [SPU-ISA p:245 s:10 Mtspr] RR opcode 0x10C; SA sits in the RA field.
        0x10C => return Ok(SpuInstruction::Mtspr { sa: ra7, rt: rt7 }),
        // [SPU-ISA p:33 s:3 Lqx] RR opcode 0x1C4.
        0x1C4 => {
            return Ok(SpuInstruction::Lqx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:37 s:3 Stqx] RR opcode 0x144.
        0x144 => {
            return Ok(SpuInstruction::Stqx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x1A8 => return Ok(SpuInstruction::Bi { ra: ra7 }),
        0x201 => return Ok(SpuInstruction::Nop),
        0x001 => return Ok(SpuInstruction::Lnop),
        // [SPU-ISA p:242 s:10 Sync] RR opcode 0x002; bit 11 is the C feature bit.
        0x002 => {
            return Ok(SpuInstruction::Sync {
                c: raw & 0x0010_0000 != 0,
            })
        }
        // [SPU-ISA p:150 s:7 Heq] RR opcode 0x3D8; RT is a false target.
        0x3D8 => return Ok(SpuInstruction::Heq { ra: ra7, rb: rb7 }),
        // [SPU-ISA p:152 s:7 Hgt] RR opcode 0x258.
        0x258 => return Ok(SpuInstruction::Hgt { ra: ra7, rb: rb7 }),
        // [SPU-ISA p:154 s:7 Hlgt] RR opcode 0x2D8.
        0x2D8 => return Ok(SpuInstruction::Hlgt { ra: ra7, rb: rb7 }),
        // [SPU-ISA p:192 s:8 Hbr] RR opcode 0x1AC; the P bit selects hbrp on the same opcode.
        0x1AC => return Ok(SpuInstruction::Hbr),
        // [SPU-ISA p:83 s:5 Clz] RR opcode 0x2A5; RB field unused.
        0x2A5 => return Ok(SpuInstruction::Clz { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:84 s:5 Cntb] RR opcode 0x2B4; RB field unused.
        0x2B4 => return Ok(SpuInstruction::Cntb { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:85 s:5 Fsmb] RR opcode 0x1B6; RB field unused.
        0x1B6 => return Ok(SpuInstruction::Fsmb { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:86 s:5 Fsmh] RR opcode 0x1B5; RB field unused.
        0x1B5 => return Ok(SpuInstruction::Fsmh { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:87 s:5 Fsm] RR opcode 0x1B4; RB field unused.
        0x1B4 => return Ok(SpuInstruction::Fsm { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:88 s:5 Gbb] RR opcode 0x1B2; RB field unused.
        0x1B2 => return Ok(SpuInstruction::Gbb { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:90 s:5 Gb] RR opcode 0x1B0; RB field unused.
        0x1B0 => return Ok(SpuInstruction::Gb { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:89 s:5 Gbh] RR opcode 0x1B1; RB field unused.
        0x1B1 => return Ok(SpuInstruction::Gbh { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:243 s:10 Dsync] RR opcode 0x003.
        0x003 => return Ok(SpuInstruction::Dsync),
        // [SPU-ISA p:249 s:11 Rchcnt] RR opcode 0x00F; CA in the RA field.
        0x00F => {
            return Ok(SpuInstruction::Rchcnt {
                rt: rt7,
                channel: ra7,
            })
        }
        // [SPU-ISA p:186 s:7 Biz] RR opcodes 0x128..0x12B; the D/E interrupt bits at [12:13] are not modeled.
        0x128 => return Ok(SpuInstruction::Biz { rt: rt7, ra: ra7 }),
        0x129 => return Ok(SpuInstruction::Binz { rt: rt7, ra: ra7 }),
        0x12A => return Ok(SpuInstruction::Bihz { rt: rt7, ra: ra7 }),
        0x12B => return Ok(SpuInstruction::Bihnz { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:41 s:3 Cbx] RR opcodes 0x1D4..0x1D7: cbx, chx, cwx, cdx.
        0x1D4 => {
            return Ok(SpuInstruction::Cbx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x1D5 => {
            return Ok(SpuInstruction::Chx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x1D6 => {
            return Ok(SpuInstruction::Cwx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        0x1D7 => {
            return Ok(SpuInstruction::Cdx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
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
        // [SPU-ISA p:91 s:5 Avgb] RR opcode 0x0D3.
        0x0D3 => {
            return Ok(SpuInstruction::Avgb {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:92 s:5 Absdb] RR opcode 0x053.
        0x053 => {
            return Ok(SpuInstruction::Absdb {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:93 s:5 Sumb] RR opcode 0x253.
        0x253 => {
            return Ok(SpuInstruction::Sumb {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:72 s:5 Mpy] RR opcode 0x3C4.
        0x3C4 => {
            return Ok(SpuInstruction::Mpy {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:73 s:5 Mpyu] RR opcode 0x3CC.
        0x3CC => {
            return Ok(SpuInstruction::Mpyu {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:77 s:5 Mpyh] RR opcode 0x3C5.
        0x3C5 => {
            return Ok(SpuInstruction::Mpyh {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:78 s:5 Mpys] RR opcode 0x3C7.
        0x3C7 => {
            return Ok(SpuInstruction::Mpys {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:79 s:5 Mpyhh] RR opcode 0x3C6.
        0x3C6 => {
            return Ok(SpuInstruction::Mpyhh {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:80 s:5 Mpyhha] RR opcode 0x346.
        0x346 => {
            return Ok(SpuInstruction::Mpyhha {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:81 s:5 Mpyhhu] RR opcode 0x3CE.
        0x3CE => {
            return Ok(SpuInstruction::Mpyhhu {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:82 s:5 Mpyhhau] RR opcode 0x34E.
        0x34E => {
            return Ok(SpuInstruction::Mpyhhau {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:66 s:5 Addx] RR opcode 0x340.
        0x340 => {
            return Ok(SpuInstruction::Addx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:67 s:5 Cg] RR opcode 0x0C2.
        0x0C2 => {
            return Ok(SpuInstruction::Cg {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:68 s:5 Cgx] RR opcode 0x342.
        0x342 => {
            return Ok(SpuInstruction::Cgx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:69 s:5 Sfx] RR opcode 0x341.
        0x341 => {
            return Ok(SpuInstruction::Sfx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:70 s:5 Bg] RR opcode 0x042.
        0x042 => {
            return Ok(SpuInstruction::Bg {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:71 s:5 Bgx] RR opcode 0x343.
        0x343 => {
            return Ok(SpuInstruction::Bgx {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:58 s:5 Ah] RR opcode 0x0C8.
        0x0C8 => {
            return Ok(SpuInstruction::Ah {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:62 s:5 Sfh] RR opcode 0x048.
        0x048 => {
            return Ok(SpuInstruction::Sfh {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:98 s:5 Andc] RR opcode 0x2C1.
        0x2C1 => {
            return Ok(SpuInstruction::Andc {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:103 s:5 Orc] RR opcode 0x2C9.
        0x2C9 => {
            return Ok(SpuInstruction::Orc {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:108 s:5 Xor] RR opcode 0x241.
        0x241 => {
            return Ok(SpuInstruction::Xor {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:112 s:5 Nand] RR opcode 0x0C9.
        0x0C9 => {
            return Ok(SpuInstruction::Nand {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:114 s:5 Eqv] RR opcode 0x249.
        0x249 => {
            return Ok(SpuInstruction::Eqv {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:107 s:5 Orx] RR opcode 0x1F0; RB field unused.
        0x1F0 => return Ok(SpuInstruction::Orx { rt: rt7, ra: ra7 }),
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
        // [SPU-ISA p:97 s:5 And] RR opcode 0x0C1.
        0x0C1 => {
            return Ok(SpuInstruction::And {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:102 s:5 Or] RR opcode 0x041.
        0x041 => {
            return Ok(SpuInstruction::Or {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:94 s:5 Xsbh] RR opcode 0x2B6; RB field unused.
        0x2B6 => return Ok(SpuInstruction::Xsbh { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:95 s:5 Xshw] RR opcode 0x2AE; RB field unused.
        0x2AE => return Ok(SpuInstruction::Xshw { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:96 s:5 Xswd] RR opcode 0x2A6; RB field unused.
        0x2A6 => return Ok(SpuInstruction::Xswd { rt: rt7, ra: ra7 }),
        // [SPU-ISA p:118 s:6 Shlh] RR opcode 0x05F.
        0x05F => {
            return Ok(SpuInstruction::Shlh {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:127 s:6 Roth] RR opcode 0x05C.
        0x05C => {
            return Ok(SpuInstruction::Roth {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:136 s:6 Rothm] RR opcode 0x05D.
        0x05D => {
            return Ok(SpuInstruction::Rothm {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:145 s:6 Rotmah] RR opcode 0x05E.
        0x05E => {
            return Ok(SpuInstruction::Rotmah {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:129 s:6 Rot] RR opcode 0x058.
        0x058 => {
            return Ok(SpuInstruction::Rot {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:138 s:6 Rotm] RR opcode 0x059.
        0x059 => {
            return Ok(SpuInstruction::Rotm {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:147 s:6 Rotma] RR opcode 0x05A.
        0x05A => {
            return Ok(SpuInstruction::Rotma {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:122 s:6 Shlqbi] RR opcode 0x1DB.
        0x1DB => {
            return Ok(SpuInstruction::Shlqbi {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:134 s:6 Rotqbi] RR opcode 0x1D8.
        0x1D8 => {
            return Ok(SpuInstruction::Rotqbi {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:143 s:6 Rotqmbi] RR opcode 0x1D9.
        0x1D9 => {
            return Ok(SpuInstruction::Rotqmbi {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:120 s:6 Shl] RR opcode 0x05B.
        0x05B => {
            return Ok(SpuInstruction::Shl {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:172 s:7 Clgt] RR opcode 0x2C0.
        0x2C0 => {
            return Ok(SpuInstruction::Clgt {
                rt: rt7,
                ra: ra7,
                rb: rb7,
            })
        }
        // [SPU-ISA p:181 s:7 Bisl] RR opcode 0x1A9; the D/E interrupt bits at [12:13] are not modeled.
        0x1A9 => return Ok(SpuInstruction::Bisl { rt: rt7, ra: ra7 }),
        _ => {}
    }

    // RI7: 7-bit immediate shares bit position with rb in RR format.
    let i7 = rb7;
    match op11 {
        0x1FF => {
            return Ok(SpuInstruction::Shlqbyi {
                rt: rt7,
                ra: ra7,
                imm: i7 & 0x1F,
            })
        }
        // [SPU-ISA p:132 s:6 Rotqbyi] RI7 opcode 0x1FC.
        0x1FC => {
            return Ok(SpuInstruction::Rotqbyi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:141 s:6 Rotqmbyi] RI7 opcode 0x1FD.
        0x1FD => {
            return Ok(SpuInstruction::Rotqmbyi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:42 s:3 Chd] RI7 opcode 0x1F5.
        0x1F5 => {
            return Ok(SpuInstruction::Chd {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:46 s:3 Cdd] RI7 opcode 0x1F7.
        0x1F7 => {
            return Ok(SpuInstruction::Cdd {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:119 s:6 Shlhi] RI7 opcode 0x07F.
        0x07F => {
            return Ok(SpuInstruction::Shlhi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:128 s:6 Rothi] RI7 opcode 0x07C.
        0x07C => {
            return Ok(SpuInstruction::Rothi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:137 s:6 Rothmi] RI7 opcode 0x07D.
        0x07D => {
            return Ok(SpuInstruction::Rothmi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:146 s:6 Rotmahi] RI7 opcode 0x07E.
        0x07E => {
            return Ok(SpuInstruction::Rotmahi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:130 s:6 Roti] RI7 opcode 0x078.
        0x078 => {
            return Ok(SpuInstruction::Roti {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:123 s:6 Shlqbii] RI7 opcode 0x1FB.
        0x1FB => {
            return Ok(SpuInstruction::Shlqbii {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:135 s:6 Rotqbii] RI7 opcode 0x1F8.
        0x1F8 => {
            return Ok(SpuInstruction::Rotqbii {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:144 s:6 Rotqmbii] RI7 opcode 0x1F9.
        0x1F9 => {
            return Ok(SpuInstruction::Rotqmbii {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:121 s:6 Shli] RI7 opcode 0x07B.
        0x07B => {
            return Ok(SpuInstruction::Shli {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:139 s:6 Rotmi] RI7 opcode 0x079.
        0x079 => {
            return Ok(SpuInstruction::Rotmi {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        // [SPU-ISA p:148 s:6 Rotmai] RI7 opcode 0x07A.
        0x07A => {
            return Ok(SpuInstruction::Rotmai {
                rt: rt7,
                ra: ra7,
                imm: i7,
            })
        }
        _ => {}
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
        // [SPU-ISA p:99 s:5 Andbi] RI10 opcode 0x16.
        0x16 => {
            return Ok(SpuInstruction::Andbi {
                rt: rt7,
                ra: ra7,
                imm: (i10 & 0xFF) as u8,
            })
        }
        // [SPU-ISA p:100 s:5 Andhi] RI10 opcode 0x15.
        0x15 => {
            return Ok(SpuInstruction::Andhi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:104 s:5 Orbi] RI10 opcode 0x06.
        0x06 => {
            return Ok(SpuInstruction::Orbi {
                rt: rt7,
                ra: ra7,
                imm: (i10 & 0xFF) as u8,
            })
        }
        // [SPU-ISA p:105 s:5 Orhi] RI10 opcode 0x05.
        0x05 => {
            return Ok(SpuInstruction::Orhi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:109 s:5 Xorbi] RI10 opcode 0x46.
        0x46 => {
            return Ok(SpuInstruction::Xorbi {
                rt: rt7,
                ra: ra7,
                imm: (i10 & 0xFF) as u8,
            })
        }
        // [SPU-ISA p:110 s:5 Xorhi] RI10 opcode 0x45.
        0x45 => {
            return Ok(SpuInstruction::Xorhi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:111 s:5 Xori] RI10 opcode 0x44.
        0x44 => {
            return Ok(SpuInstruction::Xori {
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
        // [SPU-ISA p:74 s:5 Mpyi] RI10 opcode 0x74.
        0x74 => {
            return Ok(SpuInstruction::Mpyi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:75 s:5 Mpyui] RI10 opcode 0x75.
        0x75 => {
            return Ok(SpuInstruction::Mpyui {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:59 s:5 Ahi] RI10 opcode 0x1D.
        0x1D => {
            return Ok(SpuInstruction::Ahi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:63 s:5 Sfhi] RI10 opcode 0x0D.
        0x0D => {
            return Ok(SpuInstruction::Sfhi {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:65 s:5 Sfi] RI10 opcode 0x0C.
        0x0C => {
            return Ok(SpuInstruction::Sfi {
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
        // [SPU-ISA p:167 s:7 Cgti] RI10 opcode 0x4C.
        0x4C => {
            return Ok(SpuInstruction::Cgti {
                rt: rt7,
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:151 s:7 Heqi] RI10 opcode 0x7F.
        0x7F => {
            return Ok(SpuInstruction::Heqi {
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:153 s:7 Hgti] RI10 opcode 0x4F.
        0x4F => {
            return Ok(SpuInstruction::Hgti {
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:155 s:7 Hlgti] RI10 opcode 0x5F.
        0x5F => {
            return Ok(SpuInstruction::Hlgti {
                ra: ra7,
                imm: sign_extend_10(i10),
            })
        }
        // [SPU-ISA p:157 s:7 Ceqbi] RI10 opcode 0x7E; only the rightmost 8 bits of I10 are compared.
        0x7E => {
            return Ok(SpuInstruction::Ceqbi {
                rt: rt7,
                ra: ra7,
                imm: (i10 & 0xFF) as u8,
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
        // [SPU-ISA p:35 s:3 Lqr] RI16 opcode 0x067.
        0x067 => {
            return Ok(SpuInstruction::Lqr {
                rt: rt7,
                imm: i16_signed,
            })
        }
        // [SPU-ISA p:39 s:3 Stqr] RI16 opcode 0x047.
        0x047 => {
            return Ok(SpuInstruction::Stqr {
                rt: rt7,
                imm: i16_signed,
            })
        }
        // [SPU-ISA p:184 s:7 Brhnz] RI16 opcode 0x046.
        0x046 => {
            return Ok(SpuInstruction::Brhnz {
                rt: rt7,
                offset: i16_offset,
            })
        }
        // [SPU-ISA p:185 s:7 Brhz] RI16 opcode 0x044.
        0x044 => {
            return Ok(SpuInstruction::Brhz {
                rt: rt7,
                offset: i16_offset,
            })
        }
        _ => {}
    }

    // RI18 (7-bit opcode, 18-bit immediate at [7:24]).
    if op7 == 0x21 {
        let imm = (raw >> 7) & 0x3FFFF;
        return Ok(SpuInstruction::Ila { rt: rt7, imm });
    }

    // [SPU-ISA p:193 s:8 Hbra] prefix 0001000 in bits [0:6], ROH in [7:8], I16 in [9:24].
    if op7 == 0x08 {
        return Ok(SpuInstruction::Hbra);
    }

    // hbrr: prefix 0001001 in bits [0:6], ROH in [7:8].
    if op7 == 0x09 {
        return Ok(SpuInstruction::Hbrr);
    }

    Err(SpuDecodeError::for_word(raw))
}

// [SPU-ISA p:32 s:3 Lqd] RI10 imm10 is sign-extended before address compute.
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

#[cfg(test)]
#[path = "tests/decode_compiler_forms_tests.rs"]
mod compiler_forms_tests;

#[cfg(test)]
#[path = "tests/decode_job_forms_tests.rs"]
mod job_forms_tests;

#[cfg(test)]
#[path = "tests/decode_quad_x_form_tests.rs"]
mod quad_x_form_tests;

#[cfg(test)]
#[path = "tests/decode_opcode_map_tests.rs"]
mod opcode_map_tests;
