//! FNV-1a hashing for state snapshots. No randomized seed; stable across
//! platforms, runs, and Rust versions.

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

    /// Return the current hash value.
    #[inline]
    pub fn finish(self) -> u64 {
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
}
