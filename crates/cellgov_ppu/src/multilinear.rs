//! Multilinear-128: a strongly universal hash over the PPU register-file lanes.
//!
//! The hash reads a state as [`LANE_COUNT`] integer lanes, each 64 bits:
//!
//! | Lanes     | Field                                                                            |
//! | ---       | ---                                                                              |
//! | 0..=31    | `gpr[0..32]`                                                                     |
//! | 32        | `lr`                                                                             |
//! | 33        | `ctr`                                                                            |
//! | 34        | `xer`                                                                            |
//! | 35        | `cr`, zero-extended                                                              |
//! | 36        | reservation tag: 0 for none, 1 for a held line                                   |
//! | 37        | reservation line address, 0 for none                                             |
//! | 38        | `fpscr`, zero-extended                                                           |
//! | 39        | `vrsave`, zero-extended                                                          |
//! | 40..=71   | `fpr[0..32]`, raw bit patterns                                                   |
//! | 72..=135  | `vr[k]`: lane `72 + 2k` is bits 0:63 (the high half), lane `73 + 2k` bits 64:127 |
//! | 136       | `vscr`, zero-extended                                                            |
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
use cellgov_ps3_abi::hw::ppu::{FPR_COUNT, GPR_COUNT, VR_COUNT};

/// Number of 64-bit lanes the hash reads from one state.
pub const LANE_COUNT: usize = LANE_VSCR + 1;

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
/// Lane of the floating-point status and control register, zero-extended.
pub const LANE_FPSCR: usize = GPR_COUNT + 6;
/// Lane of the AltiVec VR-usage mask, zero-extended.
pub const LANE_VRSAVE: usize = GPR_COUNT + 7;
/// Lane of FPR 0; FPR `k` is lane `LANE_FPR0 + k`.
pub const LANE_FPR0: usize = GPR_COUNT + 8;
/// Lane of the high half of VR 0; VR `k` takes lanes `LANE_VR0 + 2k`
/// (bits 0:63) and `LANE_VR0 + 2k + 1` (bits 64:127).
pub const LANE_VR0: usize = LANE_FPR0 + FPR_COUNT;
/// Lane of the vector status and control register, zero-extended.
pub const LANE_VSCR: usize = LANE_VR0 + 2 * VR_COUNT;

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
    0xd85f_219d_b5c5_54e1_6a27_ff80_7441_bcd2,
    0x96a5_73e9_b482_16e8_46a9_fdac_40bf_0048,
    0x3dd1_2464_a0ee_15b4_451e_5212_96a7_eea1,
    0x56e4_398a_98f8_a0fd_7b7d_c216_0e33_35a7,
    0xc679_ee0b_ebcb_1cca_928d_6f2d_7453_424e,
    0x1b38_9942_0523_4c6d_8086_d193_a6f2_b568,
    0x21c6_e266_39ac_2c65_d9dc_cac4_14d2_3c6f,
    0x91cd_6420_57e0_0235_77fc_607d_c658_9373,
    0x05b8_abe2_6dd3_aee7_12f6_436a_c376_cc66,
    0x6495_2424_897b_2307_ee8c_2baf_6343_e5c3,
    0xdc4c_613d_9eba_2304_3505_b779_6bd1_a506,
    0x8176_daf8_00a0_5f50_8bd8_ff7a_0385_cdbc,
    0x1a76_4a3c_d781_01da_be4d_15bf_6ca2_66ac,
    0xa85e_1f38_bb2d_c749_5675_9a96_8493_cd8c,
    0xf3a9_bce7_336b_d182_365b_1501_3741_519b,
    0x1f7a_44a6_b109_ac94_3521_d628_813c_b177,
    0x6a77_afab_0f7c_9370_1796_42d8_cde9_5015,
    0x5ef1_02a8_fb35_4461_f51c_5047_64ed_82f2,
    0xc584_27f0_41ce_6808_fad8_fc45_c964_3c37,
    0xcf86_82f9_a70f_a9c0_7e1b_3b75_a400_5729,
    0x992d_d867_927b_52d8_7fbd_5db1_42f6_791f,
    0x3705_95aa_cab4_adae_b139_2dbd_c5ab_61d6,
    0x9fea_7dfc_79d4_52d9_40b1_2b12_0085_641c,
    0xa192_afe3_157c_85d0_c847_729f_4e08_f3a3,
    0x6f13_84a3_06c4_1fc2_12d0_5c40_45a3_9c19,
    0x9899_202f_d20f_0841_e9c7_1918_57e7_74b8,
    0x4eea_d809_af5b_0cc3_e809_acaf_a238_64a4,
    0x4da1_edab_a1d0_f7bd_846e_b967_3349_f8e4,
    0x87ba_e55b_8603_9fe8_7f36_7b8b_d953_eff2,
    0x3884_700f_650d_04e1_bfe4_b2ab_4698_0cad,
    0xc5fc_8907_5299_106c_37b2_fa36_1ade_a7cd,
    0x7d75_d813_f048_95b4_702f_5b39_3f62_c0e0,
    0x0a3f_c775_f4ec_f37f_e4b2_3787_a352_437f,
    0xf83f_a245_c34d_6363_b99b_cf04_0786_cf50,
    0x38b6_ea0a_0e6c_9d8a_093f_dc76_776e_37e1,
    0x1a75_e6f7_6ba7_eee8_442c_dcfe_e966_0c62,
    0x22d5_8d35_116b_5e0b_87d4_a518_0f6a_3645,
    0x589f_b216_bd82_131b_91d0_31ca_d319_aec0,
    0xabec_f76a_553d_320b_b868_6cb3_4761_2dcf,
    0xfcab_6633_7c0a_77f5_ac31_8214_381e_c437,
    0x6eb7_f0fc_a244_94ae_cf42_861d_cdc8_95a9,
    0x4aba_d7a1_586d_7a91_c21b_318d_c2f4_9745,
    0xd494_74dc_2acb_d1f0_b1d4_8737_47c1_c8e1,
    0x5434_dc8c_7d01_5bf6_e1c4_8628_7511_b6a9,
    0xa861_6df6_2e89_a193_31ce_6319_498d_8347,
    0xafd0_b486_123d_6faa_e649_5f5d_1023_01eb,
    0x0dc5_1ced_17a4_3c52_8bcb_cde8_1355_ef2d,
    0x2412_af73_fdee_7cfc_c8d5_89e4_86e2_9eed,
    0x2339_0e86_6451_7f89_251a_de58_e8a6_849d,
    0xf855_5dbd_2e8f_9cb0_cb41_7c3e_ef54_f7c3,
    0x8028_f8e1_aac3_a919_10e3_1052_acf7_48a0,
    0x2d88_6c07_3b1e_1b78_9729_74d9_0df9_faee,
    0xbc1b_7b38_7968_93ba_1958_ed43_2070_e652,
    0xca5f_2971_97a1_2dcc_e025_a273_7570_4f28,
    0x4180_10a5_70a9_24fb_9828_e294_1bfc_419c,
    0x4fba_cd2f_52b8_5c1f_33dd_5b75_6211_cc67,
    0x23c8_dfdd_1db5_7ff0_32f8_1801_a1a8_e901,
    0x2688_4eac_5ada_36da_caa8_2f9b_b42e_37d4,
    0x19fb_1a74_91d6_a7d1_5aa0_243a_a357_f38e,
    0xb31d_9178_09e4_47f0_3f9c_1972_2521_5be0,
    0xdc3c_315a_1e33_c095_3dd3_99ad_533e_80ac,
    0x566f_32cc_e830_1d95_c880_1880_83d9_ba21,
    0xb9cc_357f_3b0e_7d2e_0237_d212_3a8a_8d6c,
    0xbf63_6e9a_a7cb_f6bd_d7bd_4284_c4e2_a6a7,
    0xda2e_bb47_d505_77a9_90ba_1c11_b539_087d,
    0x4499_3d31_552b_4f57_32c2_d6f8_0a8a_8898,
    0x4505_83ed_7fb5_4b19_ec2b_0b09_e50e_f3ef,
    0xd918_a0b6_e2ef_d65c_e37a_868d_9785_f572,
    0x7d1a_6118_f2b0_f37a_9e2e_3cc1_3b34_3439,
    0xefd8_2c11_212e_37e8_af89_c05c_d4fc_75ed,
    0x55bc_16bb_9697_108e_6c47_01fa_5db6_9bee,
    0x9237_3384_41da_f445_248c_f083_1e81_a5fc,
    0xacc1_3557_e77d_e273_5209_70c2_5e06_513a,
    0x6573_29cb_0298_7cab_a9b0_b336_6a4e_55a8,
    0xc4d0_6ca2_f39a_cdd4_5dce_37d6_8170_cde1,
    0x5f1e_44e7_7e18_54c9_6883_d452_d55d_f899,
    0x05c5_bd62_f106_7032_e680_b683_ce60_fab0,
    0x5dc9_da3f_286d_18b1_94b4_bf3a_b85e_d6d8,
    0xce65_f449_e3ac_c5a3_34b0_2096_42ce_a639,
    0xc14c_3c77_1d90_4827_6add_cee2_bd9c_dee5,
    0xe24e_ed13_7ffb_b613_75dd_58ef_7996_3d1b,
    0xfdb8_3ecf_6cc2_4920_7a1d_0057_c571_69fb,
    0x3392_00f4_feb6_2d07_d33f_4d4a_c884_69f4,
    0x8226_f234_e68d_fee4_320d_ef4f_2a10_5536,
    0x7786_f3b1_3aef_c159_b282_25ac_9df6_3ee2,
    0x781b_9d03_76cc_6044_05bd_0115_226c_6ab6,
    0xd302_2302_07bd_fdab_db89_8abd_8e0d_2933,
    0x9e79_a397_ba00_b9cc_89df_84a5_f000_3ee8,
    0x011f_04f2_a75f_b9be_5a58_32bb_47bc_f19e,
    0xcbdc_6d34_b7c7_534d_28a0_d62b_36f7_e211,
    0x56c4_553d_5d0b_9393_6926_f323_4c55_dbf2,
    0x13fd_156d_2818_31ab_788f_de49_3e59_653d,
    0x9844_56f3_129d_0de5_75fe_f0b6_764f_4cba,
    0x3d15_00b0_edf9_8a29_a149_d151_9fd9_7dc4,
    0x1288_259c_4a18_8588_3040_14a3_0b42_d718,
    0x7e9d_7e05_138f_2863_8379_ec73_f351_76f4,
    0x7207_6cae_dab9_cd77_933d_40d0_47d5_c211,
    0x521d_6aec_56c0_137b_4972_307f_6da2_e896,
    0x6381_fc65_071e_876d_e5eb_a2b5_b975_969a,
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
    out[LANE_FPSCR] = u64::from(fp.fpscr);
    out[LANE_VRSAVE] = u64::from(fp.vrsave);
    out[LANE_FPR0..LANE_VR0].copy_from_slice(&fp.fpr);
    for (k, &v) in fp.vr.iter().enumerate() {
        (out[LANE_VR0 + 2 * k], out[LANE_VR0 + 2 * k + 1]) = vr_lanes(v);
    }
    out[LANE_VSCR] = u64::from(fp.vscr);
    out
}

/// The high-half and low-half lanes of a vector register.
#[inline]
pub fn vr_lanes(v: u128) -> (u64, u64) {
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
