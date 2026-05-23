//! Barrier identifier. Shared id type for barrier-shaped primitives
//! (barriers, mutexes, semaphores); the state machine behind the id
//! lives at the registry that owns the primitive.

/// Stable identifier for a barrier-shaped sync primitive.
///
/// No `Default`: a derived default would alias the registry's
/// first-issued id. Use `Option<BarrierId>` for "no barrier."
/// No `Ord`: opaque handles, not creation-order keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
#[display("{_0}")]
pub struct BarrierId(u64);

impl BarrierId {
    /// Construct from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: trace serialization and
    /// diagnostic output.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl crate::registry::RegistryId for BarrierId {
    fn new(raw: u64) -> Self {
        Self::new(raw)
    }
    fn raw(self) -> u64 {
        Self::raw(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn roundtrip() {
        assert_eq!(BarrierId::new(13).raw(), 13);
    }

    #[test]
    fn hash_matches_eq() {
        assert_eq!(hash(&BarrierId::new(7)), hash(&BarrierId::new(7)));
        assert_ne!(hash(&BarrierId::new(7)), hash(&BarrierId::new(8)));
    }

    #[test]
    fn copy_preserves_value() {
        let a = BarrierId::new(5);
        let b = a;
        assert_eq!(a, b);
        assert_eq!(a.raw(), 5);
    }

    #[test]
    fn max_id_roundtrips() {
        assert_eq!(BarrierId::new(u64::MAX).raw(), u64::MAX);
    }

    #[test]
    fn display_emits_raw_integer() {
        assert_eq!(format!("{}", BarrierId::new(42)), "42");
    }
}
