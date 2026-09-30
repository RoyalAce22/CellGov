//! Collision-quality tests for the SPU multilinear construction:
//!
//! - the side conditions of its proof
//! - the one-bit flips of every lane, and the two-bit flips of a named
//!   lane subset
//! - the lane and register swaps
//! - the structured input shapes
//!
//! The PPU's tests flip every pair of bits over its 38 lanes. The SPU
//! has 263 lanes, about 1.4 * 10^8 bit pairs, so the pairwise tests run
//! over [`PAIR_LANES`]: the first and last registers, which hold every
//! position a register lane takes, and every lane that is not a register.

use super::*;
use crate::state::SpuState;
use cellgov_sync::ReservedLine;
use std::collections::BTreeSet;

/// One bit of one lane.
#[derive(Clone, Copy, Debug)]
struct Flip {
    lane: usize,
    bit: u32,
}

/// The lanes the pairwise tests flip: r0, r1, r127, and every lane after
/// the registers.
const PAIR_LANES: [usize; 13] = [0, 1, 2, 3, 254, 255, 256, 257, 258, 259, 260, 261, 262];

fn flipped(base: &[u64; LANE_COUNT], flips: &[Flip]) -> [u64; LANE_COUNT] {
    let mut out = *base;
    for f in flips {
        out[f.lane] ^= 1u64 << f.bit;
    }
    out
}

fn all_flips() -> Vec<Flip> {
    (0..LANE_COUNT)
        .flat_map(|lane| (0..64).map(move |bit| Flip { lane, bit }))
        .collect()
}

fn pair_flips() -> Vec<Flip> {
    PAIR_LANES
        .iter()
        .flat_map(|&lane| (0..64).map(move |bit| Flip { lane, bit }))
        .collect()
}

/// SplitMix64 over `state`.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A lane vector whose lanes are distinct, nonzero, and carry bits in
/// every position.
fn patterned() -> [u64; LANE_COUNT] {
    let mut state = 0x5eed;
    let mut out = [0u64; LANE_COUNT];
    for lane in &mut out {
        *lane = splitmix64(&mut state) | 1;
    }
    out
}

fn bases() -> Vec<[u64; LANE_COUNT]> {
    vec![[0u64; LANE_COUNT], [u64::MAX; LANE_COUNT], patterned()]
}

// --- Part 1: the side conditions of the collision bound ---

#[test]
fn keys_are_the_indexed_key_stream_from_the_seed() {
    for (k, key) in KEYS.iter().enumerate() {
        assert_eq!(
            cellgov_mem::indexed_key(KEY_SEED, k as u64),
            *key,
            "key {k}"
        );
    }
}

#[test]
fn keys_are_distinct_and_nonzero() {
    let distinct: BTreeSet<u128> = KEYS.iter().copied().collect();
    assert_eq!(distinct.len(), KEY_COUNT, "two keys are equal");
    assert!(KEYS.iter().all(|&k| k != 0), "a key is zero");
}

#[test]
fn no_two_multiplier_keys_cancel_modulo_2_pow_65() {
    let mask = (1u128 << 65) - 1;
    let mults = &KEYS[1..];
    for (i, &a) in mults.iter().enumerate() {
        for (j, &b) in mults.iter().enumerate() {
            assert_ne!(
                a.wrapping_add(b) & mask,
                0,
                "lanes {i} and {j}: key sum is 0 mod 2^65"
            );
            if i != j {
                assert_ne!(
                    a.wrapping_sub(b) & mask,
                    0,
                    "lanes {i} and {j}: key difference is 0 mod 2^65"
                );
            }
        }
    }
}

/// Every state that sets one bit of one hashed field, plus the two
/// reservation states that differ only in the tag. Each state holds the
/// full limit register of a new context.
fn single_bit_states() -> Vec<SpuState> {
    let mut out = Vec::new();
    for bit in 0..128 {
        let v = (1u128 << bit).to_be_bytes();
        for k in 0..SPU_REG_COUNT {
            let mut s = SpuState::new();
            s.set_reg(k, v);
            out.push(s);
        }
        let mut s = SpuState::new();
        s.set_fpscr(1 << bit);
        out.push(s);
    }
    for bit in 0..32 {
        let mut s = SpuState::new();
        s.set_lslr(1 << bit);
        out.push(s);
        let mut s = SpuState::new();
        s.set_srr0(1 << bit);
        out.push(s);
    }
    let mut s = SpuState::new();
    s.set_interrupts_enabled(true);
    out.push(s);
    // Line addresses are 128-byte aligned inside the 42-bit EA space.
    for bit in 7..42 {
        let mut s = SpuState::new();
        s.set_reservation(Some(ReservedLine::containing(1 << bit)));
        out.push(s);
    }
    let mut s = SpuState::new();
    s.set_reservation(Some(ReservedLine::containing(0)));
    out.push(s);
    out.push(SpuState::new());
    out
}

#[test]
fn lane_encoding_is_injective_over_single_bit_states() {
    let states = single_bit_states();
    let vectors: BTreeSet<[u64; LANE_COUNT]> =
        states.iter().map(|s| lanes(&s.fingerprint())).collect();
    assert_eq!(vectors.len(), states.len());
}

#[test]
fn no_reservation_and_a_reservation_of_line_zero_differ() {
    let none = SpuState::new();
    let mut zero = SpuState::new();
    zero.set_reservation(Some(ReservedLine::containing(0)));
    let (a, b) = (lanes(&none.fingerprint()), lanes(&zero.fingerprint()));
    assert_ne!(a, b);
    assert_ne!(hash(&a), hash(&b));
}

#[test]
fn lanes_place_each_field_at_its_index() {
    let mut s = SpuState::new();
    for k in 0..SPU_REG_COUNT {
        let hi = 0x1000 + k as u128;
        let lo = 0x2000 + k as u128;
        s.set_reg(k, ((hi << 64) | lo).to_be_bytes());
    }
    s.set_fpscr((7u128 << 64) | 9);
    s.set_lslr(0x3_ffff);
    s.set_interrupts_enabled(true);
    s.set_srr0(0x40);
    s.set_reservation(Some(ReservedLine::containing(0x3000_1080)));
    let l = lanes(&s.fingerprint());
    for k in 0..SPU_REG_COUNT {
        assert_eq!(l[2 * k], 0x1000 + k as u64, "r{k} high");
        assert_eq!(l[2 * k + 1], 0x2000 + k as u64, "r{k} low");
    }
    assert_eq!(l[256..], [7, 9, 0x3_ffff, 1, 0x40, 1, 0x3000_1080]);
}

// --- Part 2: structural tests over every base ---

#[test]
fn every_single_bit_flip_changes_the_hash() {
    for (b, base) in bases().iter().enumerate() {
        let h = hash(base);
        for f in all_flips() {
            assert_ne!(hash(&flipped(base, &[f])), h, "base {b}: {f:?}");
        }
    }
}

/// The accumulator change of each flip in `flips` over `base`, as
/// [`accumulate`] computes it.
fn deltas(base: &[u64; LANE_COUNT], flips: &[Flip]) -> Vec<u128> {
    let acc = accumulate(base);
    flips
        .iter()
        .map(|&f| accumulate(&flipped(base, &[f])).wrapping_sub(acc))
        .collect()
}

/// The index pairs into `flips` that the direct check rehashes:
///
/// - every cross-lane pair of bit-63 flips
/// - a fixed spread of the rest
fn rehashed_pairs(flips: &[Flip]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..flips.len() {
        for j in i + 1..flips.len() {
            let bit63 = flips[i].bit == 63 && flips[j].bit == 63;
            if bit63 || (i * 7919 + j) % 1031 == 0 {
                out.push((i, j));
            }
        }
    }
    out
}

#[test]
fn every_two_bit_flip_of_the_pair_lanes_changes_the_hash() {
    let flips = pair_flips();
    for (b, base) in bases().iter().enumerate() {
        let acc = accumulate(base);
        let h = finish(acc);
        let d = deltas(base, &flips);
        // The delta sums below equal the accumulator of the flipped
        // vector only if `accumulate` is linear; the rehash checks that.
        for (i, j) in rehashed_pairs(&flips) {
            let direct = accumulate(&flipped(base, &[flips[i], flips[j]]));
            assert_eq!(
                direct,
                acc.wrapping_add(d[i]).wrapping_add(d[j]),
                "base {b}: {:?} + {:?} is not the sum of its deltas",
                flips[i],
                flips[j]
            );
            assert_ne!(
                finish(direct),
                h,
                "base {b}: {:?} + {:?}",
                flips[i],
                flips[j]
            );
        }
        for i in 0..flips.len() {
            let acc_i = acc.wrapping_add(d[i]);
            for j in i + 1..flips.len() {
                assert_ne!(
                    finish(acc_i.wrapping_add(d[j])),
                    h,
                    "base {b}: {:?} + {:?}",
                    flips[i],
                    flips[j]
                );
            }
        }
    }
}

#[test]
fn bit_63_of_two_lanes_is_a_named_case() {
    let pairs = rehashed_pairs(&pair_flips());
    let named = pairs
        .iter()
        .filter(|&&(i, j)| i % 64 == 63 && j % 64 == 63)
        .count();
    assert_eq!(named, PAIR_LANES.len() * (PAIR_LANES.len() - 1) / 2);
}

#[test]
fn every_lane_swap_changes_the_hash() {
    let base = patterned();
    let h = hash(&base);
    for i in 0..LANE_COUNT {
        for j in i + 1..LANE_COUNT {
            let mut swapped = base;
            swapped.swap(i, j);
            assert_ne!(hash(&swapped), h, "lanes {i} and {j}");
        }
    }
}

#[test]
fn every_register_swap_of_a_state_changes_the_hash() {
    let mut s = SpuState::new();
    let values = patterned();
    let reg = |k: usize| {
        ((u128::from(values[2 * k]) << 64) | u128::from(values[2 * k + 1])).to_be_bytes()
    };
    for k in 0..SPU_REG_COUNT {
        s.set_reg(k, reg(k));
    }
    let h = s.state_hash();
    let mut swaps = 0;
    for i in 0..SPU_REG_COUNT {
        for j in i + 1..SPU_REG_COUNT {
            s.set_reg(i, reg(j));
            s.set_reg(j, reg(i));
            assert_ne!(s.state_hash(), h, "r{i} <-> r{j}");
            s.set_reg(i, reg(i));
            s.set_reg(j, reg(j));
            swaps += 1;
        }
    }
    assert_eq!(s.state_hash(), h);
    assert_eq!(swaps, 128 * 127 / 2);
}

// --- Part 3: structured shapes, pairwise ---

fn assert_pairwise_distinct(mut hashes: Vec<u64>, what: &str) {
    let n = hashes.len();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(hashes.len(), n, "{what}: {} collision(s)", n - hashes.len());
}

#[test]
fn nearly_zero_vectors_hash_pairwise_distinct() {
    let zero = [0u64; LANE_COUNT];
    let acc = accumulate(&zero);
    let flips = pair_flips();
    let d = deltas(&zero, &flips);
    let mut hashes = vec![finish(acc)];
    for i in 0..flips.len() {
        let acc_i = acc.wrapping_add(d[i]);
        hashes.push(finish(acc_i));
        for dj in &d[i + 1..] {
            hashes.push(finish(acc_i.wrapping_add(*dj)));
        }
    }
    assert_eq!(hashes.len(), 1 + 832 + 832 * 831 / 2);
    assert_pairwise_distinct(hashes, "vectors with at most two bits set");
}

#[test]
fn counter_sequences_hash_pairwise_distinct() {
    const STEPS: u64 = 1 << 18;
    // The low half of r3 counting, SRR0 counting, and the high half of
    // r0 counting from a nonzero base.
    let mut hashes = Vec::new();
    for (lane, start) in [(7, 0), (LANE_SRR0, 1), (0, 0x1_0000_0000)] {
        let mut v = [0u64; LANE_COUNT];
        for n in 1..=STEPS {
            v[lane] = start + n;
            hashes.push(hash(&v));
        }
    }
    hashes.push(hash(&[0u64; LANE_COUNT]));
    assert_pairwise_distinct(hashes, "counter sequences");
}

#[test]
fn state_hash_is_the_multilinear_hash_of_its_lanes() {
    for s in &single_bit_states() {
        assert_eq!(s.state_hash(), hash(&lanes(&s.fingerprint())));
    }
}

// --- The scheme id ---

#[test]
fn scheme_id_wire_format_golden() {
    assert_eq!(SCHEME_ID, 0x11d4_57ad_88c9_9134);
}

#[test]
fn a_change_to_any_one_key_moves_the_scheme_id() {
    for i in 0..KEY_COUNT {
        let mut keys = KEYS;
        keys[i] ^= 1 << 77;
        assert_ne!(scheme_id(SCHEME_TAG, &keys), SCHEME_ID, "key {i}");
    }
}

#[test]
fn a_new_tag_version_moves_the_scheme_id() {
    assert_ne!(scheme_id(b"cellgov-spu-multilinear/v2", &KEYS), SCHEME_ID);
}
