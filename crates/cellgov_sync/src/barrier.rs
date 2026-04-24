//! Barrier identifier. Shared id type for barrier-shaped primitives
//! (barriers, mutexes, semaphores); the state machine behind the id
//! lives at the registry that owns the primitive.

/// Stable identifier for a barrier-shaped sync primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BarrierId(u64);

impl BarrierId {
    /// Construct a `BarrierId` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(BarrierId::new(13).raw(), 13);
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(BarrierId::default(), BarrierId::new(0));
    }

    #[test]
    fn ordering_is_total() {
        assert!(BarrierId::new(8) < BarrierId::new(9));
        assert_eq!(BarrierId::new(2), BarrierId::new(2));
    }
}
