//! Minimal SPU instruction encoder for microtest SPU programs.
//!
//! All instructions are 32-bit big-endian.

#![allow(dead_code)]

/// Encode `ilhu $rt, imm16` (immediate load halfword upper).
pub fn ilhu(rt: u32, imm: u16) -> u32 {
    (0x082 << 23) | ((imm as u32) << 7) | rt
}

/// Encode `iohl $rt, imm16` (immediate OR halfword lower).
pub fn iohl(rt: u32, imm: u16) -> u32 {
    (0x0C1 << 23) | ((imm as u32) << 7) | rt
}

/// Encode `il $rt, imm16` (immediate load word, sign-extended).
pub fn il(rt: u32, imm: i16) -> u32 {
    (0x081 << 23) | ((imm as u16 as u32) << 7) | rt
}

/// Encode `wrch $ca, $rt` (write channel).
pub fn wrch(ca: u32, rt: u32) -> u32 {
    (0x105 << 21) | (ca << 7) | rt
}

/// Encode `rdch $rt, $ca` (read channel).
pub fn rdch(rt: u32, ca: u32) -> u32 {
    (0x00D << 21) | (ca << 7) | rt
}

/// Encode `stqd $rt, imm($ra)` (store quadword d-form).
///
/// `imm` is in bytes and must be a multiple of 16; the I10 field stores `imm / 16`
/// as a signed 10-bit value.
pub fn stqd(rt: u32, ra: u32, imm: i16) -> u32 {
    let i10 = ((imm / 16) as u16 as u32) & 0x3FF;
    (0x24 << 24) | (i10 << 14) | (ra << 7) | rt
}

/// Encode `stop` (stop and signal with code 0).
pub fn stop() -> u32 {
    0x00000000
}

/// Encode a sequence of SPU instructions into big-endian bytes.
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
    fn ilhu_encodes_correctly() {
        let inst = ilhu(3, 0x1337);
        assert_eq!(inst.to_be_bytes(), [0x41, 0x09, 0x9B, 0x83]);
    }

    #[test]
    fn iohl_encodes_correctly() {
        let inst = iohl(3, 0xBAAD);
        assert_eq!(inst.to_be_bytes(), [0x60, 0xDD, 0x56, 0x83]);
    }

    #[test]
    fn il_encodes_correctly() {
        let inst = il(3, 4);
        assert_eq!(inst.to_be_bytes(), [0x40, 0x80, 0x02, 0x03]);
    }

    #[test]
    fn wrch_encodes_correctly() {
        let inst = wrch(16, 3);
        assert_eq!(inst.to_be_bytes(), [0x20, 0xA0, 0x08, 0x03]);
    }

    #[test]
    fn rdch_encodes_correctly() {
        let inst = rdch(9, 24);
        assert_eq!(inst.to_be_bytes(), [0x01, 0xA0, 0x0C, 0x09]);
    }
}
