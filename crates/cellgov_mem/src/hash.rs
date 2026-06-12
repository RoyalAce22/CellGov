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
#[path = "tests/hash_tests.rs"]
mod tests;
