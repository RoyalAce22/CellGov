//! Multilinear-128: a strongly universal hash over the PPU register-file lanes.
//!
//! The hash reads a state as [`LANE_COUNT`] integer lanes, each 64 bits:
//!
//! | Lanes  | Field                                          |
//! | ---    | ---                                            |
//! | 0..=31 | `gpr[0..32]`                                   |
//! | 32     | `lr`                                           |
//! | 33     | `ctr`                                          |
//! | 34     | `xer`                                          |
//! | 35     | `cr`, zero-extended                            |
//! | 36     | reservation tag: 0 for none, 1 for a held line |
//! | 37     | reservation line address, 0 for none           |
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
//! stay true. The middle one is a check on the committed keys:
//!
//! - The lane encoding is fixed-length and injective. The reservation
//!   takes two lanes, so no reservation and a reservation of line 0 give
//!   different vectors.
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
//!
//! # Why the accumulator is 128 bits
//!
//! With a 64-bit accumulator and 64-bit keys, the theorem covers one
//! output bit. Two registers that each change in bit 63 change the sum
//! by `(k_a + k_b) * 2^63`, which is 0 modulo 2^64 when `k_a` and `k_b`
//! have the same parity. [`accumulate_with`] adds the terms.
//! [Black1999 p:13 s:4.3] For the related NH family, an XOR in place of
//! the inner or the outer addition increases the collision probability
//! by a large amount.

use cellgov_exec::PpuFingerprint;
use cellgov_ps3_abi::hw::ppu::GPR_COUNT;

/// Number of 64-bit lanes the hash reads from one state.
pub const LANE_COUNT: usize = 38;

/// Lane of the link register; GPR `k` is lane `k`.
pub const LANE_LR: usize = GPR_COUNT;
/// Lane of the count register.
pub const LANE_CTR: usize = GPR_COUNT + 1;
/// Lane of the fixed-point exception register.
pub const LANE_XER: usize = GPR_COUNT + 2;
/// Lane of the condition register, zero-extended.
pub const LANE_CR: usize = GPR_COUNT + 3;
/// Lane of the reservation tag: 0 for none, 1 for a held line.
pub const LANE_RESERVATION_TAG: usize = GPR_COUNT + 4;
/// Lane of the reservation line address, 0 for none.
pub const LANE_RESERVATION_LINE: usize = GPR_COUNT + 5;

/// Number of keys: one additive key and one multiplier per lane.
pub const KEY_COUNT: usize = LANE_COUNT + 1;

/// The seed of the SplitMix64 stream that [`KEYS`] comes from.
pub const KEY_SEED: u64 = 0;

/// The committed keys: `derive_keys(KEY_SEED)`.
///
/// `KEYS[0]` is the additive key, and `KEYS[i + 1]` multiplies lane `i`.
/// Each of these changes the hash scheme:
///
/// - a change to a key
/// - a change to the key order
/// - a change to the lane order
pub const KEYS: [u128; KEY_COUNT] = [
    0xe220_a839_7b1d_cdaf_6e78_9e6a_a1b9_65f4,
    0x06c4_5d18_8009_454f_f88b_b8a8_724c_81ec,
    0x1b39_896a_51a8_749b_53cb_9f0c_747e_a2ea,
    0x2c82_9abe_1f45_32e1_c584_133a_c916_ab3c,
    0x3ee5_7890_41c9_8ac3_f3b8_488c_368c_b0a6,
    0x657e_ecdd_3cb1_3d09_c2d3_26e0_055b_def6,
    0x8621_a03f_e0bb_db7b_8e1f_7555_983a_a92f,
    0xb54e_0f16_00cc_4d19_84bb_3f97_971d_80ab,
    0x7d29_825c_7552_1255_c3cf_1710_2b7f_7f86,
    0x3466_e9a0_8391_4f64_d81a_8d2b_5a44_85ac,
    0xdb01_602b_100b_9ed7_a903_8a92_1825_f10d,
    0xedf5_f1d9_0dca_2f6a_5449_6ad6_7bd2_634c,
    0xdd7c_01d4_f540_7269_935e_82f1_db4c_4f7b,
    0x69b8_2ebc_9223_3300_40d2_9eb5_7de1_d510,
    0xa2f0_9dab_b45c_6316_ee52_1d7a_0f4d_3872,
    0xf169_52ee_72f3_454f_377d_35de_a8e4_0225,
    0x0c7d_e806_4963_bab0_0558_2d37_111a_c529,
    0xd254_741f_599d_c6f7_6963_0f75_93d1_08c3,
    0x417e_f961_81da_a383_3c3c_41a3_b433_43a1,
    0x6e19_905d_cbe5_31df_4fa9_fa73_2485_1729,
    0x84eb_4454_a792_922a_134f_7096_9181_75ce,
    0x07dc_930b_3022_78a8_12c0_15a9_7019_e937,
    0xcc06_c316_52eb_f438_ecee_6563_0a69_1e37,
    0x3e84_ecb1_763e_79ad_690e_d476_743a_ae49,
    0x7746_15d7_b1a1_f2e1_22b3_53f0_4f4f_52da,
    0xe3dd_d86b_a71a_5eb1_df26_8ade_b651_3356,
    0x2098_eb73_d436_7d77_03d6_8453_23ce_3c71,
    0xc952_c562_0043_c714_9b19_6bca_844f_1705,
    0x3026_0345_dd9e_0ec1_cf44_8a58_82bb_9698,
    0xf4a5_78dc_cbc8_7656_bfde_aed9_a17b_3c8f,
    0xed79_402d_1d5c_5d7b_55f0_70ab_1cbb_f170,
    0x3e00_a349_29a8_8f1d_e255_b237_b8bb_18fb,
    0x2a7b_67af_6c6a_d50e_466d_5e7f_3e46_f143,
    0x4237_5cb3_99a4_fc72_8c8a_1f14_8a8b_b259,
    0x32fc_ab5d_aed5_bdfc_9e60_398c_8d85_53c0,
    0xee89_cceb_8c40_64c0_db02_1594_1d86_a66f,
    0x5ccd_e782_03c3_67a8_f1bc_bc6a_1ec1_1786,
    0xef05_4fce_ee95_4551_df82_012d_0555_c6df,
    0x2925_66ff_7240_3c08_c4dd_302a_1bfa_1137,
];

/// The domain tag [`SCHEME_ID`] starts from.
///
/// The derivation sees the lane count and the keys, but not [`lanes`],
/// [`accumulate_with`] or [`finish`]. A change to one of those needs a
/// new version suffix when the lane count and the keys stay the same.
pub const SCHEME_TAG: &[u8] = b"cellgov-ppu-multilinear/v1";

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

/// The next output of a SplitMix64 stream, whose state is `state`.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A key set from the SplitMix64 stream that starts at `seed`.
///
/// Key `i` is output `2i` in its high half and output `2i + 1` in its
/// low half. [`KEYS`] is the set for [`KEY_SEED`].
pub fn derive_keys(seed: u64) -> [u128; KEY_COUNT] {
    let mut state = seed;
    let mut keys = [0u128; KEY_COUNT];
    for key in &mut keys {
        let hi = splitmix64(&mut state);
        let lo = splitmix64(&mut state);
        *key = (u128::from(hi) << 64) | u128::from(lo);
    }
    keys
}

/// The lanes of the state that `fp` describes, in the order of the
/// module table.
pub fn lanes(fp: &PpuFingerprint) -> [u64; LANE_COUNT] {
    let mut out = [0u64; LANE_COUNT];
    out[..GPR_COUNT].copy_from_slice(&fp.gpr);
    out[LANE_LR] = fp.lr;
    out[LANE_CTR] = fp.ctr;
    out[LANE_XER] = fp.xer;
    out[LANE_CR] = u64::from(fp.cr);
    (out[LANE_RESERVATION_TAG], out[LANE_RESERVATION_LINE]) =
        reservation_lanes(fp.reservation_line);
    out
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

#[cfg(test)]
#[path = "tests/multilinear_indexed_key_tests.rs"]
mod indexed_key_tests;
