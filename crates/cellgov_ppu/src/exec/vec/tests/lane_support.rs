//! Vector lane packing shared by the vector tests.

/// Pack four words into a vector, word 0 most significant.
pub(super) fn pack_u32x4(lanes: [u32; 4]) -> u128 {
    let mut r = [0u8; 16];
    for (i, v) in lanes.iter().enumerate() {
        r[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

/// Element `i` of `v` with `w`-bit elements, element 0 most significant.
pub(super) fn lane(v: u128, w: u32, i: u32) -> u64 {
    let lanes = 128 / w;
    ((v >> ((lanes - 1 - i) * w)) as u64) & ((1u64 << w) - 1)
}

/// `v` with `x` ORed into element `i` of `w`-bit elements.
pub(super) fn with_lane(v: u128, w: u32, i: u32, x: u64) -> u128 {
    let lanes = 128 / w;
    v | (u128::from(x) << ((lanes - 1 - i) * w))
}

/// Operand classes for `w`-bit elements: 0, 1, all ones (unsigned max
/// and signed -1), the signed max and min, and a fixed arbitrary value.
pub(super) fn operand_classes(w: u32) -> [u64; 6] {
    let mask = (1u64 << w) - 1;
    let sign = 1u64 << (w - 1);
    [0, 1, mask, sign - 1, sign, 0x9E37_79B9 & mask]
}

/// A vA, a vB, and the element pairs they hold, element 0 first.
pub(super) type ClassPairVector = (u128, u128, Vec<(u64, u64)>);

/// Every ordered pair of [`operand_classes`], packed `128 / w` pairs per
/// vector pair.
pub(super) fn class_pair_vectors(w: u32) -> Vec<ClassPairVector> {
    let classes = operand_classes(w);
    let pairs: Vec<(u64, u64)> = classes
        .iter()
        .flat_map(|&a| classes.iter().map(move |&b| (a, b)))
        .collect();
    pairs
        .chunks((128 / w) as usize)
        .map(|chunk| {
            let (mut a, mut b) = (0u128, 0u128);
            for (i, &(x, y)) in (0..).zip(chunk) {
                a = with_lane(a, w, i, x);
                b = with_lane(b, w, i, y);
            }
            (a, b, chunk.to_vec())
        })
        .collect()
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
