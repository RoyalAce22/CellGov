//! FNV-1a hashing for state snapshots. No randomized seed; the output
//! is stable across platforms, runs, and Rust versions for a given
//! byte sequence.
//!
//! Multi-byte values must be serialized in a fixed byte order before
//! being fed in -- the byte stream is the contract, not the in-memory
//! representation. CellGov uses little-endian for state-hash payloads
//! (see `sync_state_hash` for the canonical layout).

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
    pub const fn new() -> Self {
        Self { state: FNV_OFFSET }
    }

    /// Feed bytes into the hash.
    #[inline]
    pub const fn write(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            self.state ^= bytes[i] as u64;
            self.state = self.state.wrapping_mul(FNV_PRIME);
            i += 1;
        }
    }

    /// Return the current hash value. Borrows so an in-progress hash
    /// can be observed without consuming the hasher.
    ///
    /// This type is not a `core::hash::Hasher`: that trait's default
    /// `write_u32` / `write_u64` / `write_usize` call `to_ne_bytes`,
    /// which differs between LE and BE hosts. Callers feeding
    /// multi-byte values must serialize them in a fixed byte order
    /// (CellGov uses little-endian) before invoking [`Self::write`].
    #[inline]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for Fnv1aHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// The SplitMix64 state increment.
const SPLITMIX64_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

/// The SplitMix64 finalizer of one state.
#[inline]
pub(crate) const fn splitmix64_mix(state: u64) -> u64 {
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Key `k` of a Multilinear-128 key stream, computed in O(1).
///
/// The stream is the SplitMix64 stream that starts at `seed`: key `k`
/// holds output `2k + 1` in its high half and output `2k + 2` in its low
/// half, where output `n` is the finalizer of `seed + n * GAMMA`
/// (wrapping). A table built by walking the stream in order holds the
/// same key at index `k`. `2k` wraps, so key `k + 2^63` equals key `k`.
///
/// A lane space with no fixed size, such as one lane per unit id, takes
/// its keys from here instead of from a table.
#[inline]
pub const fn indexed_key(seed: u64, k: u64) -> u128 {
    let n = k.wrapping_mul(2);
    let hi = splitmix64_mix(seed.wrapping_add(n.wrapping_add(1).wrapping_mul(SPLITMIX64_GAMMA)));
    let lo = splitmix64_mix(seed.wrapping_add(n.wrapping_add(2).wrapping_mul(SPLITMIX64_GAMMA)));
    ((hi as u128) << 64) | lo as u128
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/indexed_key_tests.rs"]
mod indexed_key_tests;
