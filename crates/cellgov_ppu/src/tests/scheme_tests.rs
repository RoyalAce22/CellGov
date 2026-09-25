//! The two state-hash scheme ids: their values, and what moves them.

use super::*;
use crate::multilinear::{scheme_id, KEYS, KEY_COUNT, SCHEME_ID, SCHEME_TAG};

#[test]
fn scheme_id_wire_format_golden() {
    assert_eq!(FNV1A_SCHEME_ID, 0x3cab_a2a8_eabe_81d0);
    assert_eq!(SCHEME_ID, 0x7e54_7695_1ed5_a8b7);
}

#[test]
fn the_state_hash_records_the_fnv1a_scheme() {
    assert_eq!(STATE_HASH_SCHEME, FNV1A_SCHEME_ID);
    assert_ne!(SCHEME_ID, FNV1A_SCHEME_ID);
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
fn a_swap_of_two_keys_moves_the_scheme_id() {
    let mut keys = KEYS;
    keys.swap(1, 2);
    assert_ne!(scheme_id(SCHEME_TAG, &keys), SCHEME_ID);
}

#[test]
fn a_new_tag_version_moves_the_scheme_id() {
    assert_ne!(scheme_id(b"cellgov-ppu-multilinear/v2", &KEYS), SCHEME_ID);
}

/// The FNV-1a byte stream of the FNV-1a scheme: each GPR, LR, CTR and
/// XER as 8 LE bytes, CR as 4, then a reservation tag byte and, for a
/// held line, its address as 8.
fn fnv1a_scheme_hash(s: &PpuState) -> u64 {
    let fp = s.fingerprint();
    let mut h = cellgov_mem::Fnv1aHasher::new();
    for r in fp.gpr.iter().chain([&fp.lr, &fp.ctr, &fp.xer]) {
        h.write(&r.to_le_bytes());
    }
    h.write(&fp.cr.to_le_bytes());
    match fp.reservation_line {
        None => h.write(&[0]),
        Some(addr) => {
            h.write(&[1]);
            h.write(&addr.to_le_bytes());
        }
    }
    h.finish()
}

#[test]
fn the_recorded_scheme_is_the_one_state_hash_computes() {
    let mut s = PpuState::new();
    for k in 0..32 {
        s.set_gpr(k, 0x9e37_79b9_7f4a_7c15u64.wrapping_mul(k as u64 + 1));
    }
    s.set_cr(0x2400_0042);
    s.set_reservation(Some(cellgov_sync::ReservedLine::containing(0x3000_1080)));
    let multilinear = crate::multilinear::hash(&crate::multilinear::lanes(&s.fingerprint()));
    match STATE_HASH_SCHEME {
        SCHEME_ID => assert_eq!(s.state_hash(), multilinear),
        FNV1A_SCHEME_ID => {
            assert_eq!(s.state_hash(), fnv1a_scheme_hash(&s));
            assert_ne!(s.state_hash(), multilinear);
        }
        other => panic!("STATE_HASH_SCHEME 0x{other:016x} names no known scheme"),
    }
}
