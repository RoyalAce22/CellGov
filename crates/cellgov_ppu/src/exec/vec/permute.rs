//! Bit select, byte permute and the double-vector byte shift.

pub(super) fn vsel(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-133 s:6.2] per-bit mux: c_bit ? b_bit : a_bit.
    (a & !c) | (b & c)
}

pub(super) fn vperm(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-112 s:6.2] low 5 bits of each c byte index the
    // a:b concatenation (32 bytes).
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let cb = c.to_be_bytes();
    let mut concat = [0u8; 32];
    concat[0..16].copy_from_slice(&ab);
    concat[16..32].copy_from_slice(&bb);
    let mut r = [0u8; 16];
    for i in 0..16 {
        let idx = (cb[i] & 0x1F) as usize;
        r[i] = concat[idx];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vsldoi(a: u128, b: u128, sh: u8) -> u128 {
    // [AltiVec-PEM p:6-136 s:6.2] SH is 4-bit; left-shift the a:b
    // concatenation by `sh` bytes and return the high 16.
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut concat = [0u8; 32];
    concat[0..16].copy_from_slice(&ab);
    concat[16..32].copy_from_slice(&bb);
    let shift = (sh & 0xF) as usize;
    let mut r = [0u8; 16];
    r.copy_from_slice(&concat[shift..shift + 16]);
    u128::from_be_bytes(r)
}
