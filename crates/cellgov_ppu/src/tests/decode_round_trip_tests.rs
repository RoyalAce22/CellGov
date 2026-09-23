//! Decode -> re-encode round trip preserving XO-form Rc and OE bits.

use super::*;
use crate::instruction::encode::encode;

// -- Round-trip tripwire --
//
// Catches the rldimi-as-rldicl class of mis-route structurally: if
// decode produced the wrong variant, re-encoding picks the wrong
// sub-opcode and the round-trip diverges. The vectors are hand-built
// words whose Rc / OE bits are the ones most likely to drop.

#[test]
fn round_trip_preserves_xo_form_rc_and_oe() {
    // Vectors: every combination of Rc and OE where applicable,
    // across the XO-form arithmetic, X-form logical and shift,
    // M-form and MD-form rotates, and FP. Each entry is raw u32.
    // Primary 31, RT=5, RA=6, RB=7 where possible. Dot/oe toggles
    // are the bits most likely to silently drop.
    let xo9_ops = [266u32, 40, 235, 233, 138, 491, 459, 489, 457]; // add,subf,mullw,mulld,adde,divw,divwu,divd,divdu
    let mut vectors: Vec<u32> = Vec::new();
    for &xo in &xo9_ops {
        for oe in [0u32, 1] {
            for rc in [0u32, 1] {
                vectors.push(
                    (31u32 << 26)
                        | (5u32 << 21)
                        | (6u32 << 16)
                        | (7u32 << 11)
                        | (oe << 10)
                        | (xo << 1)
                        | rc,
                );
            }
        }
    }
    // addze / neg: no RB slot.
    for &xo in &[202u32, 104] {
        for oe in [0u32, 1] {
            for rc in [0u32, 1] {
                vectors.push(
                    (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (oe << 10) | (xo << 1) | rc,
                );
            }
        }
    }
    // mulh family: xo_9 only, no OE bit meaningful.
    for &xo in &[11u32, 75, 9, 73] {
        for rc in [0u32, 1] {
            vectors
                .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc);
        }
    }
    // X-form logical + shift (use RB=7).
    for &xo in &[444u32, 412, 28, 60, 124, 316, 24, 536, 27, 539, 792, 794] {
        for rc in [0u32, 1] {
            vectors
                .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (xo << 1) | rc);
        }
    }
    // cntlz + extsb/h/w: reserved RB slot is zero in canonical encodings.
    for &xo in &[26u32, 58, 922, 954, 986] {
        for rc in [0u32, 1] {
            vectors.push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (xo << 1) | rc);
        }
    }
    // srawi: SH in RB slot.
    for rc in [0u32, 1] {
        vectors
            .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (12u32 << 11) | (824u32 << 1) | rc);
    }
    // sradi: XS-form. SH=34 (hi=1, lo=2): sh_lo=2 at bits 11..15, sh_hi=1 at bit 1.
    for rc in [0u32, 1] {
        vectors.push(
            (31u32 << 26)
                | (5u32 << 21)
                | (6u32 << 16)
                | (2u32 << 11)
                | (413u32 << 2)
                | (1u32 << 1)
                | rc,
        );
        // SH=3 (hi=0, lo=3).
        vectors
            .push((31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (3u32 << 11) | (413u32 << 2) | rc);
    }
    // M-form: rlwimi, rlwinm, rlwnm with sh=4, mb=8, me=20.
    for primary in [20u32, 21] {
        for rc in [0u32, 1] {
            vectors.push(
                (primary << 26)
                    | (5u32 << 21)
                    | (6u32 << 16)
                    | (4u32 << 11)
                    | (8u32 << 6)
                    | (20u32 << 1)
                    | rc,
            );
        }
    }
    for rc in [0u32, 1] {
        vectors.push(
            (23u32 << 26)
                | (5u32 << 21)
                | (6u32 << 16)
                | (7u32 << 11)
                | (8u32 << 6)
                | (20u32 << 1)
                | rc,
        );
    }
    // MD-form rotates. mask=33 (hi=1, lo=1), sh=34 (hi=1, lo=2).
    for xo in 0..=3u32 {
        for rc in [0u32, 1] {
            vectors.push(
                (30u32 << 26)
                    | (5u32 << 21)
                    | (6u32 << 16)
                    | (2u32 << 11)
                    | (1u32 << 6)
                    | (1u32 << 5)
                    | (xo << 2)
                    | (1u32 << 1)
                    | rc,
            );
        }
    }
    // dcbz: RA=6, RB=7.
    vectors.push((31u32 << 26) | (6u32 << 16) | (7u32 << 11) | (1014u32 << 1));

    // FP primary 59 and 63: xo=21 (fadd), xo=25 (fmul low 5), Rc=0/1.
    for &primary in &[59u32, 63] {
        for &xo in &[21u32, 50] {
            for rc in [0u32, 1] {
                vectors.push(
                    (primary << 26)
                        | (5u32 << 21)
                        | (6u32 << 16)
                        | (7u32 << 11)
                        | (2u32 << 6)
                        | (xo << 1)
                        | rc,
                );
            }
        }
    }

    assert!(!vectors.is_empty(), "round-trip vectors must not be empty");
    for raw in vectors {
        let decoded =
            decode(raw).unwrap_or_else(|e| panic!("decode failed for {raw:#010x}: {e:?}"));
        let reencoded = encode(&decoded).unwrap_or_else(|e| {
            panic!("encoder refused decoded={decoded:?} (raw={raw:#010x}): {e}")
        });
        assert_eq!(
            reencoded, raw,
            "round-trip mismatch: raw={raw:#010x} decoded={decoded:?} re-encoded={reencoded:#010x}",
        );
    }
}
