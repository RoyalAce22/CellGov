//! `StateHash` -- a deterministic hash of some piece of runtime state,
//! captured at a controlled checkpoint for replay comparison.
//!
//! Four checkpoint targets are supported: committed memory, runnable
//! queue, sync object state, and unit status. Each produces its own
//! `StateHash` value. Replay tooling compares pairs of values: identical
//! hashes mean the underlying state is bit-for-bit identical (modulo
//! hash collisions, which `u64` is wide enough to make negligible at
//! the scale of one runtime instance).
//!
//! `StateHash` is intentionally algorithm-agnostic. It carries a `u64`
//! and nothing more; the producer of the hash decides which algorithm
//! to use, as long as it is deterministic across hosts and runs. No
//! specific algorithm is currently pinned. The trace writer ultimately
//! serializes hashes in stable byte order so that text rendering tools
//! can compare them across builds.

/// A 64-bit deterministic hash of some piece of runtime state.
///
/// `StateHash` is `Copy + Eq + Hash + Ord`. It is the value the trace
/// records at every state checkpoint and the value replay tooling
/// compares when it asserts equivalence between two runs of the same
/// scenario. There is no `From<u64>` impl on purpose: producing a state
/// hash is a deliberate operation, and ad-hoc construction outside the
/// hash producer should be visible at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StateHash(u64);

impl StateHash {
    /// The zero hash. Useful as the empty-state sentinel and as a
    /// `Default`.
    pub const ZERO: Self = Self(0);

    /// Construct a `StateHash` from a raw 64-bit value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying 64-bit value.
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
