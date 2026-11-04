//! Narrow the elements of vA followed by vB to half their width.
//!
//! Each returns, beside the result, whether any element saturated; the
//! modulo forms and `vpkpx` always return false.

use super::integer::{clamp_signed, clamp_unsigned};

/// The `w`-bit elements of vA then vB, each narrowed by `f` to `w / 2`
/// bits and placed in order; the flag is set when `f` reports
/// saturation for any element.
fn narrow(a: u128, b: u128, w: u32, f: impl Fn(u64) -> (u64, bool)) -> (u128, bool) {
    let per = 128 / w;
    let mask = (1u64 << w) - 1;
    let mut r = 0u128;
    let mut sat = false;
    for (k, v) in [a, b].into_iter().enumerate() {
        for i in 0..per {
            let element = (v >> (128 - w * (i + 1))) as u64 & mask;
            let (n, s) = f(element);
            let slot = k as u32 * per + i;
            r |= u128::from(n) << (128 - (w / 2) * (slot + 1));
            sat |= s;
        }
    }
    (r, sat)
}

fn signed(x: u64, w: u32) -> i64 {
    ((x << (64 - w)) as i64) >> (64 - w)
}

/// `vpkuhum`: the low byte of each half word.
pub(super) fn vpkuhum(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 16, |x| (x & 0xFF, false))
}

/// `vpkuwum`: the low half word of each word.
pub(super) fn vpkuwum(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 32, |x| (x & 0xFFFF, false))
}

/// `vpkuhus`: each unsigned half word clamped to an unsigned byte.
pub(super) fn vpkuhus(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 16, |x| clamp_unsigned(x as i64, 8))
}

/// `vpkuwus`: each unsigned word clamped to an unsigned half word.
pub(super) fn vpkuwus(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 32, |x| clamp_unsigned(x as i64, 16))
}

/// `vpkshus`: each signed half word clamped to an unsigned byte.
pub(super) fn vpkshus(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 16, |x| clamp_unsigned(signed(x, 16), 8))
}

/// `vpkswus`: each signed word clamped to an unsigned half word.
pub(super) fn vpkswus(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 32, |x| clamp_unsigned(signed(x, 32), 16))
}

/// `vpkshss`: each signed half word clamped to a signed byte.
pub(super) fn vpkshss(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 16, |x| clamp_signed(signed(x, 16), 8))
}

/// `vpkswss`: each signed word clamped to a signed half word.
pub(super) fn vpkswss(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 32, |x| clamp_signed(signed(x, 32), 16))
}

/// `vpkpx`: each word as a 1:5:5:5 pixel of its bits 7, 8:12, 16:20 and
/// 24:28.
pub(super) fn vpkpx(a: u128, b: u128) -> (u128, bool) {
    narrow(a, b, 32, |x| {
        let pixel = ((x >> 24) & 1) << 15
            | ((x >> 19) & 0x1F) << 10
            | ((x >> 11) & 0x1F) << 5
            | ((x >> 3) & 0x1F);
        (pixel, false)
    })
}
