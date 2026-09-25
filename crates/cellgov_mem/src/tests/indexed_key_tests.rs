//! The indexed Multilinear-128 key against keys from an in-order walk of
//! the SplitMix64 stream.

use super::*;

/// Keys 0 and 1 of the seed-0 stream: the first two PPU state-hash keys.
#[test]
fn indexed_key_wire_format_golden() {
    assert_eq!(indexed_key(0, 0), 0xe220_a839_7b1d_cdaf_6e78_9e6a_a1b9_65f4);
    assert_eq!(indexed_key(0, 1), 0x06c4_5d18_8009_454f_f88b_b8a8_724c_81ec);
}

#[test]
fn indexed_key_matches_an_in_order_walk() {
    for seed in [0, 1, 0x6365_6c6c_6d65_6d31, u64::MAX] {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(SPLITMIX64_GAMMA);
            splitmix64_mix(state)
        };
        for k in 0..600 {
            let hi = u128::from(next());
            let lo = u128::from(next());
            assert_eq!(
                indexed_key(seed, k),
                (hi << 64) | lo,
                "seed {seed:#x} key {k}"
            );
        }
    }
}

#[test]
fn indexed_key_wraps_at_the_top_of_the_index_space() {
    assert_ne!(indexed_key(0, u64::MAX), indexed_key(0, u64::MAX - 1));
    assert_ne!(indexed_key(0, u64::MAX), 0);
    assert_eq!(indexed_key(0, 5 + (1 << 63)), indexed_key(0, 5));
}
