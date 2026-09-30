//! Multilinear-128: a strongly universal hash over the SPU state lanes.
//!
//! The PPU state hash uses the same construction, with its own lanes and
//! keys. The hash reads a state as [`LANE_COUNT`] integer lanes, each 64
//! bits:
//!
//! | Lanes    | Field                                          |
//! | ---      | ---                                            |
//! | 0..=255  | `regs[k]`: high half in lane `2k`, low in `2k + 1` |
//! | 256      | `fpscr`, high half                             |
//! | 257      | `fpscr`, low half                              |
//! | 258      | `lslr`, zero-extended                          |
//! | 259      | interrupt enable: 0 or 1                       |
//! | 260      | `srr0`, zero-extended                          |
//! | 261      | reservation tag: 0 for none, 1 for a held line |
//! | 262      | reservation line address, 0 for none           |
//!
//! It then computes
//!
//! ```text
//! acc  = KEYS[0] + sum over i of KEYS[i + 1] * lane[i]      (mod 2^128)
//! hash = acc >> 64
//! ```
//!
//! # Collision bound
//!
//! [LemireKaser2014 p:4 s:3] Theorem 3.1 with K = 128 and L = 64: over a
//! uniform draw of the keys, the family `acc >> 63` is strongly universal
//! over fixed-length lane vectors, with K - L + 1 = 65 output bits.
//!
//! [LemireKaser2014 p:2 s:1] Strong universality over a set of output
//! bits holds over each subset of those bits, so `acc >> 64` keeps it.
//!
//! Thus two distinct lane vectors give the same hash with a probability
//! of at most 2^-64. Over N compared pairs the union bound gives
//! N * 2^-64.
//!
//! The bound holds only while the first and the last of these conditions
//! stay true. The middle one is a check on the keys:
//!
//! - The lane encoding is fixed-length and injective. Each 128-bit value
//!   takes two lanes, and the reservation takes two lanes, so no
//!   reservation and a reservation of line 0 give different vectors.
//! - The keys are distinct and nonzero. [LemireKaser2014 p:4 s:3] The
//!   theorem draws each key uniformly and does not need this; the check
//!   rejects a fixed key that ignores a lane or makes two lanes
//!   interchangeable. The tests also check that for each pair of
//!   multiplier keys, the sum and the difference are nonzero modulo
//!   2^65. This excludes, for these keys, the case where two lanes that
//!   each change in bit 63 cancel.
//! - No state that the runtime produces depends on the keys. The keys
//!   are fixed, and the bound is over the key draw. A fixed key keeps the
//!   bound while nothing that picks the input knows the key
//!   [CarterWegman1979 p:147 s:Properties of Universal Classes]. So a
//!   hash value must not steer which states the runtime produces.

use cellgov_exec::SpuFingerprint;
use cellgov_ps3_abi::hw::spu::SPU_REG_COUNT;

/// Number of 64-bit lanes the hash reads from one state.
pub const LANE_COUNT: usize = 2 * SPU_REG_COUNT + 7;

/// Lane of the FPSCR's high half; its low half is the next lane.
pub const LANE_FPSCR: usize = 2 * SPU_REG_COUNT;
/// Lane of the local storage limit register.
pub const LANE_LSLR: usize = LANE_FPSCR + 2;
/// Lane of the interrupt-enable state.
pub const LANE_INTERRUPTS_ENABLED: usize = LANE_FPSCR + 3;
/// Lane of state save and restore register 0.
pub const LANE_SRR0: usize = LANE_FPSCR + 4;
/// Lane of the reservation tag: 0 for none, 1 for a held line.
pub const LANE_RESERVATION_TAG: usize = LANE_FPSCR + 5;
/// Lane of the reservation line address, 0 for none.
pub const LANE_RESERVATION_LINE: usize = LANE_FPSCR + 6;

/// Number of keys: one additive key and one multiplier per lane.
pub const KEY_COUNT: usize = LANE_COUNT + 1;

/// The seed of the SplitMix64 stream that [`KEYS`] comes from.
pub const KEY_SEED: u64 = 0x6365_6c6c_7370_7531;

/// The keys: key `k` of the [`cellgov_mem::indexed_key`] stream from
/// [`KEY_SEED`].
///
/// `KEYS[0]` is the additive key, and `KEYS[i + 1]` multiplies lane `i`.
/// Each of these changes the hash scheme:
///
/// - a change to the seed
/// - a change to the lane order
pub const KEYS: [u128; KEY_COUNT] = {
    let mut keys = [0u128; KEY_COUNT];
    let mut k = 0;
    while k < KEY_COUNT {
        keys[k] = cellgov_mem::indexed_key(KEY_SEED, k as u64);
        k += 1;
    }
    keys
};

/// The domain tag [`SCHEME_ID`] starts from.
///
/// The derivation sees the lane count and the keys, but not [`lanes`],
/// [`accumulate_with`] or [`finish`]. A change to one of those needs a
/// new version suffix when the lane count and the keys stay the same.
pub const SCHEME_TAG: &[u8] = b"cellgov-spu-multilinear/v1";

/// The id of the multilinear hash scheme.
pub const SCHEME_ID: u64 = scheme_id(SCHEME_TAG, &KEYS);

/// FNV-1a over `tag`, then [`LANE_COUNT`] as eight LE bytes, then each
/// key in order as sixteen LE bytes.
pub const fn scheme_id(tag: &[u8], keys: &[u128; KEY_COUNT]) -> u64 {
    let mut h = cellgov_mem::Fnv1aHasher::new();
    h.write(tag);
    h.write(&(LANE_COUNT as u64).to_le_bytes());
    let mut i = 0;
    while i < KEY_COUNT {
        h.write(&keys[i].to_le_bytes());
        i += 1;
    }
    h.finish()
}

/// The lanes of the state that `fp` describes, in the order of the
/// module table.
pub fn lanes(fp: &SpuFingerprint) -> [u64; LANE_COUNT] {
    let mut out = [0u64; LANE_COUNT];
    for (k, &reg) in fp.regs.iter().enumerate() {
        (out[2 * k], out[2 * k + 1]) = wide_lanes(reg);
    }
    (out[LANE_FPSCR], out[LANE_FPSCR + 1]) = wide_lanes(fp.fpscr);
    out[LANE_LSLR] = u64::from(fp.lslr);
    out[LANE_INTERRUPTS_ENABLED] = u64::from(fp.interrupts_enabled);
    out[LANE_SRR0] = u64::from(fp.srr0);
    (out[LANE_RESERVATION_TAG], out[LANE_RESERVATION_LINE]) =
        reservation_lanes(fp.reservation_line);
    out
}

/// The high and low lanes of a 128-bit value.
#[inline]
pub fn wide_lanes(v: u128) -> (u64, u64) {
    ((v >> 64) as u64, v as u64)
}

/// The tag and line-address lanes of a reservation.
#[inline]
pub fn reservation_lanes(line: Option<u64>) -> (u64, u64) {
    match line {
        None => (0, 0),
        Some(addr) => (1, addr),
    }
}

/// The change to an accumulator under [`KEYS`] when lane `lane` moves
/// from `old` to `new`.
///
/// The accumulator is linear in each lane modulo 2^128, so the change
/// is exact, and changes to several lanes add in any order.
#[inline]
pub fn lane_delta(lane: usize, old: u64, new: u64) -> u128 {
    KEYS[lane + 1].wrapping_mul(u128::from(new).wrapping_sub(u128::from(old)))
}

/// The change to an accumulator when the 128-bit value whose high half
/// is lane `lane` moves from `old` to `new`.
#[inline]
pub fn wide_lane_delta(lane: usize, old: u128, new: u128) -> u128 {
    let (old_hi, old_lo) = wide_lanes(old);
    let (hi, lo) = wide_lanes(new);
    lane_delta(lane, old_hi, hi).wrapping_add(lane_delta(lane + 1, old_lo, lo))
}

/// The 128-bit accumulator of `lanes` under `keys`.
pub fn accumulate_with(keys: &[u128; KEY_COUNT], lanes: &[u64; LANE_COUNT]) -> u128 {
    let mut acc = keys[0];
    for (key, &lane) in keys[1..].iter().zip(lanes) {
        acc = acc.wrapping_add(key.wrapping_mul(u128::from(lane)));
    }
    acc
}

/// The 128-bit accumulator of `lanes` under [`KEYS`].
pub fn accumulate(lanes: &[u64; LANE_COUNT]) -> u128 {
    accumulate_with(&KEYS, lanes)
}

/// The hash of an accumulator: its high 64 bits.
#[inline]
pub fn finish(acc: u128) -> u64 {
    (acc >> 64) as u64
}

/// The hash of `lanes` under [`KEYS`].
pub fn hash(lanes: &[u64; LANE_COUNT]) -> u64 {
    finish(accumulate(lanes))
}

#[cfg(test)]
#[path = "tests/multilinear_tests.rs"]
mod tests;
