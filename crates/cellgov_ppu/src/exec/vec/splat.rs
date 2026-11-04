//! Splat an element or a sign-extended 5-bit immediate across a vector.

pub(super) fn vspltw(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let start = (idx as usize & 3) * 4;
    let word = u32::from_be_bytes([bb[start], bb[start + 1], bb[start + 2], bb[start + 3]]);
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        r[i..i + 4].copy_from_slice(&word.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

pub(super) fn vspltisw(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm as i8 | !0x1F_u8 as i8) as i32
    } else {
        imm as i32
    };
    let word = (val as u32).to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        r[i..i + 4].copy_from_slice(&word);
    }
    u128::from_be_bytes(r)
}

pub(super) fn vspltb(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let byte = bb[idx as usize & 0xF];
    u128::from_be_bytes([byte; 16])
}

pub(super) fn vsplth(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let start = (idx as usize & 7) * 2;
    let half = u16::from_be_bytes([bb[start], bb[start + 1]]);
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        let bytes = half.to_be_bytes();
        r[i] = bytes[0];
        r[i + 1] = bytes[1];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vspltisb(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm | 0xE0) as i8
    } else {
        imm as i8
    };
    u128::from_be_bytes([val as u8; 16])
}

pub(super) fn vspltish(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm as i8 | !0x1F_u8 as i8) as i16
    } else {
        imm as i16
    };
    let half = (val as u16).to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        r[i] = half[0];
        r[i + 1] = half[1];
    }
    u128::from_be_bytes(r)
}
