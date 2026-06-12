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
#[path = "tests/barrier_tests.rs"]
mod tests;
