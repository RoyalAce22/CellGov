//! Collision-quality tests for the multilinear construction:
//!
//! - the side conditions of its proof
//! - the exhaustive one- and two-bit flips
//! - the lane swaps
//! - the structured input shapes

use super::*;
use crate::state::PpuState;
use cellgov_sync::ReservedLine;
use std::collections::BTreeSet;

/// One bit of one lane.
#[derive(Clone, Copy, Debug)]
struct Flip {
    lane: usize,
    bit: u32,
}

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

/// Lane vectors of PPU states that a title boot passed through, as
/// `boot run --state-hash-census` sampled them at dispatches 2^20,
/// 40 * 2^20 and 100 * 2^20.
const TITLE_STATES: [[u64; LANE_COUNT]; 3] = [
    [
        0x73,
        0xd00ff860,
        0x823fb8,
        0x1,
        0x73e38e,
        0x24,
        0x813578,
        0xc64ea8,
        0x7f7f7f7f7f7f7f7f,
        0x66,
        0x7f7f7f7f7f7f7f7f,
        0x6970706c655f7363,
        0x2,
        0x10407030,
        0x0,
        0x0,
        0x0,
        0x0,
        0x24,
        0x100,
        0x813578,
        0xc64dd0,
        0xc64cd0,
        0xc64cd0,
        0x20,
        0xc64da8,
        0xd8,
        0xc64da8,
        0xc64da8,
        0xc64db0,
        0x806cb4,
        0xc64da0,
        0x69a190,
        0x69a178,
        0x0,
        0x24000044,
        0x0,
        0x0,
    ],
    [
        0x1,
        0xd00ffaa0,
        0x823fb8,
        0xc66848,
        0xc66520,
        0xc66b90,
        0xc66b80,
        0xc66b90,
        0x7f7f7f7f7f7f7f7f,
        0xc66850,
        0x733620,
        0x845ed8,
        0x953e78,
        0x10407030,
        0x0,
        0x0,
        0x0,
        0x0,
        0x12f,
        0x190,
        0xc66520,
        0xc66b78,
        0xc66850,
        0xc66520,
        0xc669e0,
        0xc66848,
        0x813578,
        0xc66520,
        0xc66848,
        0xc66520,
        0x806cb4,
        0xc66520,
        0x69ae8c,
        0x69a178,
        0x0,
        0x28000044,
        0x0,
        0x0,
    ],
    [
        0x5f5f706c725f7368,
        0xd00ff7f0,
        0x823fb8,
        0x72e300,
        0x734598,
        0x14d,
        0x813578,
        0xc67f30,
        0x7f7f7f7f7f7f7f7f,
        0xe4def5e4f1f2f4f2,
        0x7f7f7f7f7f7f7f7f,
        0x655f766572737573,
        0x953e78,
        0x10407030,
        0x0,
        0x0,
        0x0,
        0x0,
        0x14d,
        0x120,
        0xc679d8,
        0xc67e70,
        0xc67c30,
        0xc679d8,
        0xc67d48,
        0xc67c28,
        0x813578,
        0xc679d8,
        0xc67c28,
        0xc679d8,
        0x806cb4,
        0xc67d48,
        0x69a190,
        0x69a178,
        0x0,
        0x28000042,
        0x0,
        0x0,
    ],
];

fn bases() -> Vec<[u64; LANE_COUNT]> {
    let mut out = vec![[0u64; LANE_COUNT], [u64::MAX; LANE_COUNT], patterned()];
    out.extend_from_slice(&TITLE_STATES);
    out
}

// --- Part 1: the side conditions of the collision bound ---

#[test]
fn keys_are_the_splitmix64_stream_from_the_seed() {
    assert_eq!(derive_keys(KEY_SEED), KEYS);
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

/// Every state that sets one bit of one hashed register, plus the two
/// reservation states that differ only in the tag.
fn single_bit_states() -> Vec<PpuState> {
    let mut out = Vec::new();
    for bit in 0..64 {
        let v = 1u64 << bit;
        for k in 0..32 {
            let mut s = PpuState::new();
            s.set_gpr(k, v);
            out.push(s);
        }
        let mut s = PpuState::new();
        s.set_lr(v);
        out.push(s);
        let mut s = PpuState::new();
        s.set_ctr(v);
        out.push(s);
        let mut s = PpuState::new();
        s.set_xer(v);
        out.push(s);
    }
    for bit in 0..32 {
        let mut s = PpuState::new();
        s.set_cr(1 << bit);
        out.push(s);
    }
    // Line addresses are 128-byte aligned inside the 42-bit EA space.
    for bit in 7..42 {
        let mut s = PpuState::new();
        s.set_reservation(Some(ReservedLine::containing(1 << bit)));
        out.push(s);
    }
    let mut s = PpuState::new();
    s.set_reservation(Some(ReservedLine::containing(0)));
    out.push(s);
    out.push(PpuState::new());
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
    let none = PpuState::new();
    let mut zero = PpuState::new();
    zero.set_reservation(Some(ReservedLine::containing(0)));
    let (a, b) = (lanes(&none.fingerprint()), lanes(&zero.fingerprint()));
    assert_ne!(a, b);
    assert_ne!(hash(&a), hash(&b));
}

#[test]
fn lanes_place_each_field_at_its_index() {
    let mut s = PpuState::new();
    for k in 0..32 {
        s.set_gpr(k, 100 + k as u64);
    }
    s.set_lr(200);
    s.set_ctr(201);
    s.set_xer(202);
    s.set_cr(0xffff_ffff);
    s.set_reservation(Some(ReservedLine::containing(0x3000_1080)));
    let l = lanes(&s.fingerprint());
    for (k, &lane) in l[..32].iter().enumerate() {
        assert_eq!(lane, 100 + k as u64);
    }
    assert_eq!(l[32..], [200, 201, 202, 0xffff_ffff, 1, 0x3000_1080]);
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
fn every_two_bit_flip_changes_the_hash() {
    let flips = all_flips();
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
    let pairs = rehashed_pairs(&all_flips());
    let named = pairs
        .iter()
        .filter(|&&(i, j)| i % 64 == 63 && j % 64 == 63)
        .count();
    assert_eq!(named, LANE_COUNT * (LANE_COUNT - 1) / 2);
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
fn every_gpr_swap_of_a_state_changes_the_hash() {
    let mut s = PpuState::new();
    let values = patterned();
    for (k, &v) in values[..32].iter().enumerate() {
        s.set_gpr(k, v);
    }
    let base = lanes(&s.fingerprint());
    let h = hash(&base);
    let mut swaps = 0;
    for i in 0..32 {
        for j in i + 1..32 {
            let mut t = s.clone();
            t.set_gpr(i, values[j]);
            t.set_gpr(j, values[i]);
            assert_ne!(hash(&lanes(&t.fingerprint())), h, "r{i} <-> r{j}");
            swaps += 1;
        }
    }
    assert_eq!(swaps, 496);
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
    let flips = all_flips();
    let d = deltas(&zero, &flips);
    let mut hashes = vec![finish(acc)];
    for i in 0..flips.len() {
        let acc_i = acc.wrapping_add(d[i]);
        hashes.push(finish(acc_i));
        for dj in &d[i + 1..] {
            hashes.push(finish(acc_i.wrapping_add(*dj)));
        }
    }
    assert_eq!(hashes.len(), 1 + 2432 + 2432 * 2431 / 2);
    assert_pairwise_distinct(hashes, "vectors with at most two bits set");
}

#[test]
fn counter_sequences_hash_pairwise_distinct() {
    const STEPS: u64 = 1 << 18;
    // ctr counting, r3 counting, and lr counting from a nonzero base.
    let mut hashes = Vec::new();
    for (lane, start) in [(33, 0), (3, 1), (32, 0x1_0000_0000)] {
        let mut v = [0u64; LANE_COUNT];
        for n in 1..=STEPS {
            v[lane] = start + n;
            hashes.push(hash(&v));
        }
    }
    hashes.push(hash(&[0u64; LANE_COUNT]));
    assert_pairwise_distinct(hashes, "counter sequences");
}

// --- The FNV-1a baseline ---

/// The `PpuState::state_hash` of the state that `l` describes; `l` holds
/// only values a state can hold.
fn fnv_of_lanes(l: &[u64; LANE_COUNT]) -> u64 {
    let mut h = cellgov_mem::Fnv1aHasher::new();
    for r in &l[..35] {
        h.write(&r.to_le_bytes());
    }
    h.write(&(l[35] as u32).to_le_bytes());
    if l[36] == 0 {
        h.write(&[0u8]);
    } else {
        h.write(&[1u8]);
        h.write(&l[37].to_le_bytes());
    }
    h.finish()
}

#[test]
fn fnv_of_lanes_is_the_state_hash_of_the_state() {
    for s in single_bit_states() {
        assert_eq!(fnv_of_lanes(&lanes(&s.fingerprint())), s.state_hash());
    }
}

/// The single-bit flips a state can hold:
///
/// - every bit of the GPRs, LR, CTR and XER
/// - the 32 bits of CR
/// - the reservation tag
fn state_flips() -> Vec<Flip> {
    all_flips()
        .into_iter()
        .filter(|f| f.lane < 35 || (f.lane == 35 && f.bit < 32) || (f.lane == 36 && f.bit == 0))
        .collect()
}

/// A base vector restricted to what a state can hold: CR is 32 bits,
/// and the reservation is empty.
fn state_base(v: &[u64; LANE_COUNT]) -> [u64; LANE_COUNT] {
    let mut out = *v;
    out[35] &= 0xffff_ffff;
    out[36] = 0;
    out[37] = 0;
    out
}

#[test]
fn fnv1a_baseline_separates_every_one_and_two_bit_flip() {
    let flips = state_flips();
    for (b, base) in bases().iter().map(state_base).enumerate() {
        let h = fnv_of_lanes(&base);
        for (i, &fi) in flips.iter().enumerate() {
            assert_ne!(fnv_of_lanes(&flipped(&base, &[fi])), h, "base {b}: {fi:?}");
            for &fj in &flips[i + 1..] {
                assert_ne!(
                    fnv_of_lanes(&flipped(&base, &[fi, fj])),
                    h,
                    "base {b}: {fi:?} + {fj:?}"
                );
            }
        }
    }
}

#[test]
fn fnv1a_baseline_on_structured_shapes() {
    let zero = [0u64; LANE_COUNT];
    let flips = state_flips();
    let mut near_zero = vec![fnv_of_lanes(&zero)];
    for (i, &fi) in flips.iter().enumerate() {
        near_zero.push(fnv_of_lanes(&flipped(&zero, &[fi])));
        for &fj in &flips[i + 1..] {
            near_zero.push(fnv_of_lanes(&flipped(&zero, &[fi, fj])));
        }
    }
    assert_pairwise_distinct(near_zero, "FNV-1a: vectors with at most two bits set");
    let mut counters = vec![fnv_of_lanes(&zero)];
    for (lane, start) in [(33, 0), (3, 1), (32, 0x1_0000_0000)] {
        let mut v = zero;
        for n in 1..=(1u64 << 18) {
            v[lane] = start + n;
            counters.push(fnv_of_lanes(&v));
        }
    }
    assert_pairwise_distinct(counters, "FNV-1a: counter sequences");
}

#[test]
fn every_gpr_swap_changes_the_fnv1a_baseline() {
    let base = state_base(&patterned());
    let h = fnv_of_lanes(&base);
    for i in 0..32 {
        for j in i + 1..32 {
            let mut swapped = base;
            swapped.swap(i, j);
            assert_ne!(fnv_of_lanes(&swapped), h, "r{i} <-> r{j}");
        }
    }
}
