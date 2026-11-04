//! Vector lane packing shared by the vector tests.

/// Pack four words into a vector, word 0 most significant.
pub(super) fn pack_u32x4(lanes: [u32; 4]) -> u128 {
    let mut r = [0u8; 16];
    for (i, v) in lanes.iter().enumerate() {
        r[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

/// Unpack a vector into four single-precision values, word 0 first.
pub(super) fn unpack_f32x4(v: u128) -> [f32; 4] {
    let b = v.to_be_bytes();
    let mut r = [0.0f32; 4];
    for i in 0..4 {
        r[i] = f32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
    }
    r
}
