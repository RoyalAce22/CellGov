//! FNV-1a hashing for state snapshots. No randomized seed; the output
//! is stable across platforms, runs, and Rust versions for a given
//! byte sequence.
//!
//! Multi-byte values must be serialized in a fixed byte order before
//! being fed in -- the byte stream is the contract, not the in-memory
//! representation. CellGov uses little-endian for state-hash payloads
//! (see `PpuStateHash` and `sync_state_hash` for the canonical layouts).

/// FNV-1a offset basis (64-bit).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a prime (64-bit).
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a hash of a byte sequence. Empty input returns the offset basis.
#[inline]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Incremental FNV-1a hasher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv1aHasher {
    state: u64,
}

impl Fnv1aHasher {
    /// Create a new hasher seeded with the FNV offset basis.
    #[inline]
    pub fn new() -> Self {
        Self { state: FNV_OFFSET }
    }

    /// Feed bytes into the hash.
    #[inline]
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= b as u64;
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    /// Return the current hash value. Borrows so an in-progress hash
    /// can be observed without consuming the hasher.
    ///
    /// Deliberately does not implement `core::hash::Hasher`: the trait's
    /// default `write_u32` / `write_u64` / `write_usize` shims call
    /// `to_ne_bytes`, which would silently diverge between LE and BE
    /// hosts and undermine the determinism contract this module
    /// promises. Callers feeding multi-byte values must serialize them
    /// in a fixed byte order (CellGov uses little-endian) before
    /// invoking [`Self::write`].
    #[inline]
    pub fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for Fnv1aHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
