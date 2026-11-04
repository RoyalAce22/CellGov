//! Interleave the high or low halves of two vectors.

pub(super) fn vmrghb(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..8 {
        r[i * 2] = ab[i];
        r[i * 2 + 1] = bb[i];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vmrghh(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..4 {
        r[i * 4] = ab[i * 2];
        r[i * 4 + 1] = ab[i * 2 + 1];
        r[i * 4 + 2] = bb[i * 2];
        r[i * 4 + 3] = bb[i * 2 + 1];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vmrghw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    r[0..4].copy_from_slice(&ab[0..4]);
    r[4..8].copy_from_slice(&bb[0..4]);
    r[8..12].copy_from_slice(&ab[4..8]);
    r[12..16].copy_from_slice(&bb[4..8]);
    u128::from_be_bytes(r)
}

pub(super) fn vmrglb(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..8 {
        r[i * 2] = ab[i + 8];
        r[i * 2 + 1] = bb[i + 8];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vmrglh(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..4 {
        r[i * 4] = ab[8 + i * 2];
        r[i * 4 + 1] = ab[8 + i * 2 + 1];
        r[i * 4 + 2] = bb[8 + i * 2];
        r[i * 4 + 3] = bb[8 + i * 2 + 1];
    }
    u128::from_be_bytes(r)
}

pub(super) fn vmrglw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    r[0..4].copy_from_slice(&ab[8..12]);
    r[4..8].copy_from_slice(&bb[8..12]);
    r[8..12].copy_from_slice(&ab[12..16]);
    r[12..16].copy_from_slice(&bb[12..16]);
    u128::from_be_bytes(r)
}
