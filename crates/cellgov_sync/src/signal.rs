//! Signal notification register and its leaf identifier.
//! Block/wake translation happens upstream.
//!
//! Spec gap (deferred): the CBE-Handbook describes two notification
//! modes, OR (bits accumulate) and overwrite (last writer wins),
//! configured per-register via `SPU_Cfg` or `SPU_WrSigNotifyCfg`.
//! Only OR mode is modeled; no current SPU exec path issues
//! signal-channel writes, so the overwrite path has no producer.
//! Wire it (and a `mode: SignalMode` field) when the SPU exec layer
//! starts emitting `wrch SigNotify`; until then a guest configuring
//! overwrite mode would silently get OR-mode behaviour.
// [CBEA p:101 s:8.7] Two SPU signal-notification facilities (Sig_Notify_1, Sig_Notify_2), each one 32-bit register + channel.
// [CBE-Handbook p:546 s:19.7] OR mode accumulates producer signals into a single 32-bit register; overwrite mode is the alternative.
// [CBE-Handbook p:547 s:19.7] Channel count for signal-notification registers saturates at 1, so SPE software cannot count writes.

/// Stable identifier for a signal notification register.
///
/// No `Default`/`Ord`: opaque handles. Use `Option<SignalId>` for
/// "no signal." Storage lives in [`crate::Registry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
#[display("{_0}")]
pub struct SignalId(u64);

impl SignalId {
    /// Construct from a raw value.
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

impl crate::registry::RegistryId for SignalId {
    fn new(raw: u64) -> Self {
        Self::new(raw)
    }
    fn raw(self) -> u64 {
        Self::raw(self)
    }
}

impl crate::registry::RegistryValueHash for SignalRegister {
    fn hash_into(&self, hasher: &mut cellgov_mem::Fnv1aHasher) {
        hasher.write(&self.value.to_le_bytes());
    }
}

/// 32-bit OR-merge signal register. Idempotent, commutative,
/// monotonic-in-bits-set across repeated [`or_in`](Self::or_in)
/// calls. Overwrite mode is not modeled (see module doc); `Default`
/// is the spec's reset value (zero-valued OR-mode register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SignalRegister {
    value: u32,
}

impl SignalRegister {
    /// Construct a zero-valued register.
    #[inline]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// Construct a register pre-loaded with `value`.
    #[inline]
    pub const fn with_value(value: u32) -> Self {
        Self { value }
    }

    /// Current register value.
    #[inline]
    pub const fn value(self) -> u32 {
        self.value
    }

    /// OR `bits` into the register, returning the post-update value.
    /// OR mode only; overwrite mode is the spec's other notification
    /// mode and has no producer in the runtime yet (see module doc).
    #[inline]
    pub fn or_in(&mut self, bits: u32) -> u32 {
        self.value |= bits;
        self.value
    }

    /// Reset the register to zero.
    #[inline]
    pub fn clear(&mut self) {
        self.value = 0;
    }
}

#[cfg(test)]
#[path = "tests/signal_tests.rs"]
mod tests;
