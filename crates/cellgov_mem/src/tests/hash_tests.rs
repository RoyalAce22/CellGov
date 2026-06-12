//! FNV-1a-64 spec vectors and incremental-hasher equivalence with the oneshot.

use super::*;

#[test]
fn empty_input_returns_offset_basis() {
    assert_eq!(fnv1a(&[]), FNV_OFFSET);
}

#[test]
fn hasher_matches_oneshot() {
    let data = b"hello";
    let oneshot = fnv1a(data);
    let mut hasher = Fnv1aHasher::new();
    hasher.write(data);
    assert_eq!(hasher.finish(), oneshot);
}

#[test]
fn incremental_matches_concatenated() {
    let mut hasher = Fnv1aHasher::new();
    hasher.write(&[1, 2]);
    hasher.write(&[3, 4]);
    assert_eq!(hasher.finish(), fnv1a(&[1, 2, 3, 4]));
}

#[test]
fn different_inputs_produce_different_hashes() {
    assert_ne!(fnv1a(&[0]), fnv1a(&[1]));
}

/// Reference vectors from the FNV-1a-64 specification test suite
/// (isthe.com/chongo/src/fnv/test_fnv.c). Anchors the algorithm
/// against the spec: a constant transposition, FNV-1-vs-FNV-1a
/// inversion ("a" diverges in the third hex digit between the
/// two), or signed-vs-unsigned byte extension would silently pass
/// the consistency tests above but break here. The 0xff vector
/// specifically pins zero-extension on bytes >= 0x80.
#[test]
fn known_fnv1a_vectors() {
    assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a(b"b"), 0xaf63_df4c_8601_f1a5);
    assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    assert_eq!(fnv1a(b"Hello, world!"), 0x38d1_3341_4498_7bf4);
    assert_eq!(fnv1a(b"\xff"), 0xaf64_724c_8602_eb6e);
}

#[test]
fn empty_chunks_are_identity() {
    let mut h = Fnv1aHasher::new();
    h.write(&[]);
    h.write(b"abc");
    h.write(&[]);
    assert_eq!(h.finish(), fnv1a(b"abc"));
}

#[test]
fn finish_is_non_consuming_so_hasher_can_be_extended() {
    let mut h = Fnv1aHasher::new();
    h.write(b"ab");
    let mid = h.finish();
    h.write(b"cd");
    let full = h.finish();
    assert_eq!(mid, fnv1a(b"ab"));
    assert_eq!(full, fnv1a(b"abcd"));
}
