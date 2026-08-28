//! Known answers for `rap_to_klic` under the synthetic vault.
//!
//! The `0x05` and `0x0c` RAPs each carry the E2 subtraction through a
//! digit that reads `0x00` with a borrow pending; the reversed
//! permutation separates the digit order from the byte order, which
//! the identity permutation cannot.

use std::path::Path;

use super::*;
use crate::test_support::synthetic_vault;

/// The synthetic vault's RAP tables under the reversed permutation.
fn reversed_pbox_vault() -> KeyVault {
    let rep = |byte: u8| format!("{byte:02x}").repeat(16);
    let toml = format!(
        "rap_key = \"{}\"\nrap_pbox = \"0f0e0d0c0b0a09080706050403020100\"\n\
         rap_e1 = \"{}\"\nrap_e2 = \"{}\"\n",
        rep(0x55),
        rep(0x57),
        rep(0x58)
    );
    KeyVault::parse(Path::new("reversed-pbox.toml"), toml.as_bytes()).unwrap()
}

#[test]
fn identity_order_matches_the_frozen_answer() {
    let got = rap_to_klic(&synthetic_vault(), &[0x42u8; 16]).unwrap();
    assert_eq!(
        got,
        [
            0xD7, 0x66, 0x4F, 0x0B, 0x50, 0xA2, 0x8C, 0x48, 0xAE, 0x74, 0x37, 0xA9, 0xBB, 0x1B,
            0x40, 0xE7,
        ]
    );
}

#[test]
fn identity_order_borrows_through_a_zero_digit_like_the_frozen_answer() {
    let got = rap_to_klic(&synthetic_vault(), &[0x05u8; 16]).unwrap();
    assert_eq!(
        got,
        [
            0x04, 0x7D, 0xE0, 0x54, 0x71, 0x76, 0x25, 0xD9, 0x86, 0x8F, 0x10, 0x2D, 0x50, 0x5A,
            0x29, 0x0F,
        ]
    );
}

#[test]
fn reversed_order_matches_the_frozen_answer() {
    let got = rap_to_klic(&reversed_pbox_vault(), &[0x42u8; 16]).unwrap();
    assert_eq!(
        got,
        [
            0xC3, 0xA6, 0x54, 0xAC, 0x68, 0x3B, 0xA8, 0xB6, 0x1E, 0x41, 0xDF, 0x3A, 0xFC, 0x73,
            0x46, 0x05,
        ]
    );
}

#[test]
fn reversed_order_borrows_through_a_zero_digit_like_the_frozen_answer() {
    let got = rap_to_klic(&reversed_pbox_vault(), &[0x0Cu8; 16]).unwrap();
    assert_eq!(
        got,
        [
            0xF1, 0xE9, 0x1B, 0x47, 0x66, 0x9A, 0x44, 0x73, 0xAB, 0xD8, 0xB9, 0x52, 0x24, 0x57,
            0x79, 0xDA,
        ]
    );
}
