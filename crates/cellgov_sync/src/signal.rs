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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl core::fmt::Display for SignalId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
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
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(SignalId::new(99).raw(), 99);
    }

    #[test]
    fn display_emits_raw_integer() {
        assert_eq!(format!("{}", SignalId::new(42)), "42");
    }

    #[test]
    fn new_register_is_zero() {
        let r = SignalRegister::new();
        assert_eq!(r.value(), 0);
        assert_eq!(SignalRegister::default(), r);
    }

    #[test]
    fn with_value_constructs_pre_loaded_register() {
        let r = SignalRegister::with_value(0xdead_beef);
        assert_eq!(r.value(), 0xdead_beef);
    }

    #[test]
    fn or_in_sets_bits() {
        let mut r = SignalRegister::new();
        assert_eq!(r.or_in(0b0001), 0b0001);
        assert_eq!(r.or_in(0b0010), 0b0011);
        assert_eq!(r.or_in(0b1000), 0b1011);
        assert_eq!(r.value(), 0b1011);
    }

    #[test]
    fn or_in_is_idempotent_under_repeated_identical_updates() {
        let mut r = SignalRegister::new();
        r.or_in(0xff);
        let after_first = r.value();
        r.or_in(0xff);
        r.or_in(0xff);
        assert_eq!(r.value(), after_first);
    }

    #[test]
    fn or_in_is_monotonic_in_bits_set() {
        let mut r = SignalRegister::new();
        let mut bits_before = 0u32;
        for v in [0x01, 0x10, 0x100, 0x1000, 0x10000] {
            r.or_in(v);
            let bits_after = r.value().count_ones();
            assert!(bits_after >= bits_before);
            bits_before = bits_after;
        }
    }

    #[test]
    fn or_in_is_commutative() {
        let mut a = SignalRegister::new();
        a.or_in(0x0f);
        a.or_in(0xf0);
        let mut b = SignalRegister::new();
        b.or_in(0xf0);
        b.or_in(0x0f);
        assert_eq!(a.value(), b.value());
        assert_eq!(a.value(), 0xff);
    }

    #[test]
    fn clear_resets_to_zero() {
        let mut r = SignalRegister::with_value(0xffff_ffff);
        r.clear();
        assert_eq!(r.value(), 0);
    }

    #[test]
    fn or_in_zero_is_a_noop() {
        let mut r = SignalRegister::with_value(0xa5a5);
        let after = r.or_in(0);
        assert_eq!(after, 0xa5a5);
        assert_eq!(r.value(), 0xa5a5);
    }
}
