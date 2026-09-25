//! The shared indexed key reproduces the PPU key table.

use super::*;

#[test]
fn indexed_key_equals_every_derived_key() {
    for seed in [KEY_SEED, 1, 0x6365_6c6c_6d65_6d31, u64::MAX] {
        let keys = derive_keys(seed);
        for (k, key) in keys.iter().enumerate() {
            assert_eq!(
                cellgov_mem::indexed_key(seed, k as u64),
                *key,
                "seed {seed:#x} key {k}"
            );
        }
    }
    for (k, key) in KEYS.iter().enumerate() {
        assert_eq!(cellgov_mem::indexed_key(KEY_SEED, k as u64), *key);
    }
}
