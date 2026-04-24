//! Minimal PPC64 instruction encoder for microtest PPU wrappers.
//!
//! All instructions are 32-bit big-endian.

#![allow(dead_code)]

/// Encode `li rD, imm` (load immediate, alias for `addi rD, 0, imm`).
pub fn li(rd: u32, imm: i16) -> u32 {
    (14 << 26) | (rd << 21) | (imm as u16 as u32)
}

/// Encode `lis rD, imm` (load immediate shifted, alias for `addis rD, 0, imm`).
pub fn lis(rd: u32, imm: i16) -> u32 {
    (15 << 26) | (rd << 21) | (imm as u16 as u32)
}

/// Encode `ori rA, rS, imm` (OR immediate).
pub fn ori(ra: u32, rs: u32, imm: u16) -> u32 {
    (24 << 26) | (rs << 21) | (ra << 16) | (imm as u32)
}

/// Encode `andi. rA, rS, imm` (AND immediate, sets CR0).
pub fn andi_dot(ra: u32, rs: u32, imm: u16) -> u32 {
    (28 << 26) | (rs << 21) | (ra << 16) | (imm as u32)
}

/// Encode `lwz rD, d(rA)` (load word and zero).
pub fn lwz(rd: u32, ra: u32, d: i16) -> u32 {
    (32 << 26) | (rd << 21) | (ra << 16) | (d as u16 as u32)
}

/// Encode `lwzx rD, rA, rB` (load word and zero indexed).
pub fn lwzx(rd: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rd << 21) | (ra << 16) | (rb << 11) | (23 << 1)
}

/// Encode `stwx rS, rA, rB` (store word indexed).
pub fn stwx(rs: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (151 << 1)
}

/// Encode `stw rS, d(rA)` (store word).
pub fn stw(rs: u32, ra: u32, d: i16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | (d as u16 as u32)
}

/// Encode `sc` (system call).
pub fn sc() -> u32 {
    (17 << 26) | (1 << 1)
}

/// Encode `addi rD, rA, imm` (add immediate).
pub fn addi(rd: u32, ra: u32, imm: i16) -> u32 {
    (14 << 26) | (rd << 21) | (ra << 16) | (imm as u16 as u32)
}

/// Encode `mtctr rS` (move to count register).
pub fn mtctr(rs: u32) -> u32 {
    // mtspr SPR field is split: bits 11..15 hold SPR[0:4], bits 16..20 hold SPR[5:9].
    // CTR = 9 -> low half = 01001 in bits 16..20, high half = 0.
    (31 << 26) | (rs << 21) | (9 << 16) | (467 << 1)
}

/// Encode `bdnz offset` (branch decrement CTR, not zero).
///
/// `offset` is in bytes, relative to the current instruction.
pub fn bdnz(offset: i16) -> u32 {
    // BO=16 (0b10000): decrement CTR, branch if CTR != 0.
    let bo: u32 = 16;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bd << 2)
}

/// Encode `cmpwi crN, rA, imm` (compare word immediate), always against CR0.
pub fn cmpwi(ra: u32, imm: i16) -> u32 {
    (11 << 26) | (ra << 16) | (imm as u16 as u32)
}

/// Encode `bne offset` (branch if not equal, CR0).
///
/// `offset` is in bytes, relative to the current instruction.
pub fn bne(offset: i16) -> u32 {
    // BO=4 (branch if condition false), BI=2 (CR0[EQ]).
    let bo: u32 = 4;
    let bi: u32 = 2;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bi << 16) | (bd << 2)
}

/// Encode `beq offset` (branch if equal, CR0).
///
/// `offset` is in bytes, relative to the current instruction, must be a multiple of 4.
pub fn beq(offset: i16) -> u32 {
    // BO=12 (branch if condition true), BI=2 (CR0[EQ]).
    let bo: u32 = 12;
    let bi: u32 = 2;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bi << 16) | (bd << 2)
}

/// Encode `clrldi rA, rS, 32`, an alias for `rldicl rA, rS, 0, 32`.
pub fn clrldi(ra: u32, rs: u32) -> u32 {
    // MD-form splits the 6-bit MB field: bits 21..25 hold (MB[5] << 4) | MB[0..3],
    // and bit 30 holds SH[5]. For SH=0, MB=32 that gives bits 21..25 = 16, bit 30 = 0.
    let sh_lo: u32 = 0;
    let mb: u32 = 32;
    let mb_field = ((mb & 0x20) >> 5) | ((mb & 0x1F) << 1);
    (30 << 26) | (rs << 21) | (ra << 16) | (sh_lo << 11) | (mb_field << 5)
}

/// Encode a sequence of instructions into big-endian bytes.
pub fn encode(instructions: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(instructions.len() * 4);
    for inst in instructions {
        buf.extend_from_slice(&inst.to_be_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn li_encodes_correctly() {
        let inst = li(3, 1);
        assert_eq!(inst.to_be_bytes(), [0x38, 0x60, 0x00, 0x01]);
    }

    #[test]
    fn lis_encodes_correctly() {
        let inst = lis(3, 2);
        assert_eq!(inst.to_be_bytes(), [0x3C, 0x60, 0x00, 0x02]);
    }

    #[test]
    fn ori_encodes_correctly() {
        let inst = ori(6, 6, 8);
        assert_eq!(inst.to_be_bytes(), [0x60, 0xC6, 0x00, 0x08]);
    }

    #[test]
    fn sc_encodes_correctly() {
        let inst = sc();
        assert_eq!(inst.to_be_bytes(), [0x44, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn lwz_encodes_correctly() {
        let inst = lwz(7, 6, 0);
        assert_eq!(inst.to_be_bytes(), [0x80, 0xE6, 0x00, 0x00]);
    }

    #[test]
    fn stw_encodes_correctly() {
        let inst = stw(7, 6, 4);
        assert_eq!(inst.to_be_bytes(), [0x90, 0xE6, 0x00, 0x04]);
    }

    #[test]
    fn beq_negative_offset() {
        let inst = beq(-8);
        assert_eq!(inst.to_be_bytes()[0], 0x41);
    }

    #[test]
    fn encode_produces_big_endian_bytes() {
        let code = encode(&[li(3, 1), sc()]);
        assert_eq!(code.len(), 8);
        assert_eq!(&code[0..4], &[0x38, 0x60, 0x00, 0x01]);
        assert_eq!(&code[4..8], &[0x44, 0x00, 0x00, 0x02]);
    }
}
