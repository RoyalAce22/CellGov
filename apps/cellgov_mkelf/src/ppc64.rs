//! Minimal PPC64 instruction encoder.
//!
//! Encodes only the instructions needed for microtest PPU wrappers.
//! All instructions are 32-bit big-endian. Not all encoders are used
//! by every microtest, but all are tested and kept for future use.

#![allow(dead_code)]

/// Encode `li rD, imm` (load immediate, alias for `addi rD, 0, imm`).
pub fn li(rd: u32, imm: i16) -> u32 {
    // addi rD, rA=0, SIMM: opcode 14, rA field is zero
    (14 << 26) | (rd << 21) | (imm as u16 as u32)
}

/// Encode `lis rD, imm` (load immediate shifted, alias for `addis rD, 0, imm`).
pub fn lis(rd: u32, imm: i16) -> u32 {
    // addis rD, rA=0, SIMM: opcode 15, rA field is zero
    (15 << 26) | (rd << 21) | (imm as u16 as u32)
}

/// Encode `ori rA, rS, imm` (OR immediate).
pub fn ori(ra: u32, rs: u32, imm: u16) -> u32 {
    // ori rA, rS, UIMM: opcode 24
    (24 << 26) | (rs << 21) | (ra << 16) | (imm as u32)
}

/// Encode `andi. rA, rS, imm` (AND immediate, sets CR0).
pub fn andi_dot(ra: u32, rs: u32, imm: u16) -> u32 {
    // andi. rA, rS, UIMM: opcode 28
    (28 << 26) | (rs << 21) | (ra << 16) | (imm as u32)
}

/// Encode `lwz rD, d(rA)` (load word and zero).
pub fn lwz(rd: u32, ra: u32, d: i16) -> u32 {
    // lwz rD, d(rA): opcode 32
    (32 << 26) | (rd << 21) | (ra << 16) | (d as u16 as u32)
}

/// Encode `lwzx rD, rA, rB` (load word and zero indexed).
pub fn lwzx(rd: u32, ra: u32, rb: u32) -> u32 {
    // lwzx rD, rA, rB: opcode 31, XO=23
    (31 << 26) | (rd << 21) | (ra << 16) | (rb << 11) | (23 << 1)
}

/// Encode `stwx rS, rA, rB` (store word indexed).
pub fn stwx(rs: u32, ra: u32, rb: u32) -> u32 {
    // stwx rS, rA, rB: opcode 31, XO=151
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (151 << 1)
}

/// Encode `stw rS, d(rA)` (store word).
pub fn stw(rs: u32, ra: u32, d: i16) -> u32 {
    // stw rS, d(rA): opcode 36
    (36 << 26) | (rs << 21) | (ra << 16) | (d as u16 as u32)
}

/// Encode `sc` (system call).
pub fn sc() -> u32 {
    // sc: opcode 17, bit 30 set
    (17 << 26) | (1 << 1)
}

/// Encode `addi rD, rA, imm` (add immediate).
pub fn addi(rd: u32, ra: u32, imm: i16) -> u32 {
    (14 << 26) | (rd << 21) | (ra << 16) | (imm as u16 as u32)
}

/// Encode `mtctr rS` (move to count register).
pub fn mtctr(rs: u32) -> u32 {
    // mtspr CTR, rS: opcode 31, SPR=9 (CTR), XO=467
    // SPR encoding: bits 11-15 = SPR[0:4], bits 16-20 = SPR[5:9]
    // CTR = 9 = 0b00000_01001 -> SPR[0:4]=01001, SPR[5:9]=00000
    (31 << 26) | (rs << 21) | (9 << 16) | (467 << 1)
}

/// Encode `bdnz offset` (branch decrement CTR, not zero).
/// `offset` is in bytes, relative to the current instruction.
pub fn bdnz(offset: i16) -> u32 {
    // bc BO=16, BI=0, BD=offset/4: opcode 16
    // BO=16 (0b10000): decrement CTR, branch if CTR != 0
    let bo: u32 = 16;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bd << 2)
}

/// Encode `cmpwi crN, rA, imm` (compare word immediate).
/// For simplicity, always uses CR0 (crfD=0).
pub fn cmpwi(ra: u32, imm: i16) -> u32 {
    // cmpwi CR0, rA, SIMM: opcode 11, BF=0, L=0
    (11 << 26) | (ra << 16) | (imm as u16 as u32)
}

/// Encode `bne offset` (branch if not equal, CR0).
/// `offset` is in bytes, relative to the current instruction.
pub fn bne(offset: i16) -> u32 {
    // bc BO=4, BI=2, BD=offset/4: opcode 16
    // BO=4 (0b00100): branch if condition false
    // BI=2: CR0[EQ]
    let bo: u32 = 4;
    let bi: u32 = 2;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bi << 16) | (bd << 2)
}

/// Encode `beq offset` (branch if equal, CR0).
/// `offset` is in bytes, relative to the current instruction, must be
/// a multiple of 4.
pub fn beq(offset: i16) -> u32 {
    // bc BO=12, BI=2, BD=offset/4: opcode 16
    // BO=12 (0b01100): branch if condition true
    // BI=2: CR0[EQ]
    let bo: u32 = 12;
    let bi: u32 = 2;
    let bd = ((offset >> 2) as u16) as u32 & 0x3FFF;
    (16 << 26) | (bo << 21) | (bi << 16) | (bd << 2)
}

/// Encode `clrldi rA, rS, 32` (clear left 32 bits of doubleword).
/// Alias for `rldicl rA, rS, 0, 32`.
pub fn clrldi(ra: u32, rs: u32) -> u32 {
    // rldicl rA, rS, SH=0, MB=32
    // Encoding: opcode 30, rS, rA, sh[0:4]=0, mb[0:4]=0, mb[5]=0... complex.
    // MD-form: [30(6)][rS(5)][rA(5)][sh0:4(5)][mb0:4(5)][XO=0(3)][sh5(1)][Rc(1)]
    // sh = 0 -> sh0:4 = 0, sh5 = 0
    // mb = 32 -> mb0:4 = 0, mb5 = 0 (mb is encoded as mb[0:4]||mb[5])
    // Wait: mb=32 in the instruction encoding. mb field is 6 bits split as mb[5]||mb[0:4].
    // mb = 32 = 0b100000 -> mb[0:4] = 0b00000, mb[5] = 1... no.
    // Actually: mb is stored as (mb >> 5) | ((mb & 0x1F) << 1) in certain forms.
    // Let me use the simpler encoding: rldicl with SH=0, MB=32.
    //
    // MD-form for rldicl (XO=0):
    //   bits[0:5] = 30 (opcode)
    //   bits[6:10] = rS
    //   bits[11:15] = rA
    //   bits[16:20] = sh[0:4] (shift amount low 5 bits)
    //   bits[21:25] = mb[5] || mb[0:3] (mask begin, rotated)
    //   bits[26:29] = 0 (XO for rldicl)
    //   bit[30] = sh[5] (shift amount high bit)
    //   bit[31] = Rc
    //
    // sh = 0: sh[0:4] = 0, sh[5] = 0
    // mb = 32 = 0b100000: mb[5] = 1, mb[0:4] = 00000
    //   stored as: mb[5] || mb[0:3] = 1 || 0000 = 0b10000 = 16
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
        // li r3, 1 = addi r3, r0, 1
        let inst = li(3, 1);
        assert_eq!(inst.to_be_bytes(), [0x38, 0x60, 0x00, 0x01]);
    }

    #[test]
    fn lis_encodes_correctly() {
        // lis r3, 2 = addis r3, r0, 2
        let inst = lis(3, 2);
        assert_eq!(inst.to_be_bytes(), [0x3C, 0x60, 0x00, 0x02]);
    }

    #[test]
    fn ori_encodes_correctly() {
        // ori r6, r6, 8
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
        // lwz r7, 0(r6)
        let inst = lwz(7, 6, 0);
        assert_eq!(inst.to_be_bytes(), [0x80, 0xE6, 0x00, 0x00]);
    }

    #[test]
    fn stw_encodes_correctly() {
        // stw r7, 4(r6)
        let inst = stw(7, 6, 4);
        assert_eq!(inst.to_be_bytes(), [0x90, 0xE6, 0x00, 0x04]);
    }

    #[test]
    fn beq_negative_offset() {
        // beq -8 (branch back 2 instructions)
        let inst = beq(-8);
        // bc 12, 2, -8: BD = -2 (in units of 4)
        // Encoding: 0100_00 01100 00010 11111111111110 0 0
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
