//! Element-wise maps over a 128-bit vector register.
//!
//! Element 0 is the most significant element of the register.
//! [AltiVec-PEM p:2-4 s:2.3.1] A vector register holds 16 bytes, 8 halfwords or 4 words, numbered from the most significant end.
//!
//! The saturating forms return, beside the result, whether any element
//! saturated. The caller sets `VSCR[SAT]` from that flag.

/// Element `i` of `v` with `w`-bit elements.
pub(super) fn element(v: u128, w: u32, i: u32) -> u64 {
    let n = 128 / w;
    (v >> (w * (n - 1 - i))) as u64 & (u64::MAX >> (64 - w))
}

/// The vector of `w`-bit elements whose element `i` is the low `w` bits
/// of `f(i)`.
pub(super) fn build(w: u32, f: impl Fn(u32) -> u64) -> u128 {
    let n = 128 / w;
    (0..n).fold(0u128, |r, i| {
        r | u128::from(f(i) & (u64::MAX >> (64 - w))) << (w * (n - 1 - i))
    })
}

/// Apply `f` to each pair of byte elements.
pub(super) fn map_u8(a: u128, b: u128, f: impl Fn(u8, u8) -> u8) -> u128 {
    map_u8_sat(a, b, |x, y| (f(x, y), false)).0
}

/// Apply `f` to each pair of halfword elements.
pub(super) fn map_u16(a: u128, b: u128, f: impl Fn(u16, u16) -> u16) -> u128 {
    map_u16_sat(a, b, |x, y| (f(x, y), false)).0
}

/// Apply `f` to each pair of word elements.
pub(super) fn map_u32(a: u128, b: u128, f: impl Fn(u32, u32) -> u32) -> u128 {
    map_u32_sat(a, b, |x, y| (f(x, y), false)).0
}

/// Apply `f` to each pair of byte elements; the flag is set when `f`
/// reports saturation for any element.
pub(super) fn map_u8_sat(a: u128, b: u128, f: impl Fn(u8, u8) -> (u8, bool)) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for shift in (0..128).step_by(8).rev() {
        let (v, s) = f((a >> shift) as u8, (b >> shift) as u8);
        r |= u128::from(v) << shift;
        sat |= s;
    }
    (r, sat)
}

/// Apply `f` to each pair of halfword elements; the flag is set when
/// `f` reports saturation for any element.
pub(super) fn map_u16_sat(a: u128, b: u128, f: impl Fn(u16, u16) -> (u16, bool)) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for shift in (0..128).step_by(16).rev() {
        let (v, s) = f((a >> shift) as u16, (b >> shift) as u16);
        r |= u128::from(v) << shift;
        sat |= s;
    }
    (r, sat)
}

/// Apply `f` to each triple of halfword elements; the flag is set when
/// `f` reports saturation for any element.
pub(super) fn map3_u16_sat(
    a: u128,
    b: u128,
    c: u128,
    f: impl Fn(u16, u16, u16) -> (u16, bool),
) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for shift in (0..128).step_by(16).rev() {
        let (v, s) = f(
            (a >> shift) as u16,
            (b >> shift) as u16,
            (c >> shift) as u16,
        );
        r |= u128::from(v) << shift;
        sat |= s;
    }
    (r, sat)
}

/// Apply `f` to each triple of word elements; the flag is set when `f`
/// reports saturation for any element.
pub(super) fn map3_u32_sat(
    a: u128,
    b: u128,
    c: u128,
    f: impl Fn(u32, u32, u32) -> (u32, bool),
) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for shift in (0..128).step_by(32).rev() {
        let (v, s) = f(
            (a >> shift) as u32,
            (b >> shift) as u32,
            (c >> shift) as u32,
        );
        r |= u128::from(v) << shift;
        sat |= s;
    }
    (r, sat)
}

/// Apply `f` to each pair of word elements; the flag is set when `f`
/// reports saturation for any element.
pub(super) fn map_u32_sat(a: u128, b: u128, f: impl Fn(u32, u32) -> (u32, bool)) -> (u128, bool) {
    let mut r = 0u128;
    let mut sat = false;
    for shift in (0..128).step_by(32).rev() {
        let (v, s) = f((a >> shift) as u32, (b >> shift) as u32);
        r |= u128::from(v) << shift;
        sat |= s;
    }
    (r, sat)
}
