//! Fixed-point to floating-point conversions.

pub(super) fn vcfsx(b: u128, uimm: u8) -> u128 {
    let bytes = b.to_be_bytes();
    let mut r = [0u8; 16];
    let scale = (1u32 << (uimm & 0x1F)) as f32;
    for i in 0..4 {
        let off = i * 4;
        let v = i32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let f = (v as f32) / scale;
        r[off..off + 4].copy_from_slice(&f.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

pub(super) fn vcfux(b: u128, uimm: u8) -> u128 {
    let bytes = b.to_be_bytes();
    let mut r = [0u8; 16];
    let scale = (1u32 << (uimm & 0x1F)) as f32;
    for i in 0..4 {
        let off = i * 4;
        let v = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let f = (v as f32) / scale;
        r[off..off + 4].copy_from_slice(&f.to_be_bytes());
    }
    u128::from_be_bytes(r)
}
