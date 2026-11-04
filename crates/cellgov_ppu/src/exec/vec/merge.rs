//! Interleave the high or low halves of two vectors.

use super::lanes::{build, element};

/// The `w`-bit elements of vA and vB from element `first` on, taken in
/// turn: vA's into the even elements of vD, vB's into the odd ones.
fn merge(a: u128, b: u128, w: u32, first: u32) -> u128 {
    build(w, |i| {
        element(if i % 2 == 0 { a } else { b }, w, first + i / 2)
    })
}

pub(super) fn vmrghb(a: u128, b: u128) -> u128 {
    merge(a, b, 8, 0)
}

pub(super) fn vmrghh(a: u128, b: u128) -> u128 {
    merge(a, b, 16, 0)
}

pub(super) fn vmrghw(a: u128, b: u128) -> u128 {
    merge(a, b, 32, 0)
}

pub(super) fn vmrglb(a: u128, b: u128) -> u128 {
    merge(a, b, 8, 8)
}

pub(super) fn vmrglh(a: u128, b: u128) -> u128 {
    merge(a, b, 16, 4)
}

pub(super) fn vmrglw(a: u128, b: u128) -> u128 {
    merge(a, b, 32, 2)
}
