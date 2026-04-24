//! 64-bit state-hash wrapper used at replay-comparison checkpoints.
//!
//! The committed-memory producer (`GuestMemory::content_hash`) uses FNV-1a;
//! other checkpoint producers pick their own algorithm as long as it is
//! deterministic across hosts.

/// 64-bit deterministic hash of some piece of runtime state.
///
/// Omits `From<u64>` so hash construction stays explicit at call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StateHash(u64);

impl StateHash {
    /// Empty-state sentinel.
    pub const ZERO: Self = Self(0);

    /// Construct a `StateHash` from a raw 64-bit value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying 64-bit value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_origin() {
        assert_eq!(StateHash::ZERO, StateHash::new(0));
        assert_eq!(StateHash::default(), StateHash::ZERO);
    }

    #[test]
    fn roundtrip() {
        assert_eq!(
            StateHash::new(0xdead_beef_cafe_babe).raw(),
            0xdead_beef_cafe_babe
        );
    }

    #[test]
    fn equality_compares_value() {
        let a = StateHash::new(42);
        let b = StateHash::new(42);
        let c = StateHash::new(43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn ordering_is_total() {
        assert!(StateHash::new(1) < StateHash::new(2));
        assert!(StateHash::new(99) > StateHash::new(50));
    }

    #[test]
    fn copy_semantics_hold() {
        let h = StateHash::new(7);
        let g = h;
        assert_eq!(h, g);
    }
}
