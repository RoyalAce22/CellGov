//! Signal notification identifier and the signal-register state machine.
//!
//! `SignalId` is the leaf handle the runtime hands out at signal
//! registration time and is the payload of `Effect::SignalUpdate`.
//!
//! `SignalRegister` is the actual state. In the Cell model a signal
//! notification register is a small word other units can OR-write
//! into; the `value` is OR-written into the register and the runtime
//! applies the OR at commit time. This module owns that register.
//! The register does not produce block/wake conditions on its own;
//! the commit pipeline and event queue translate update and wait
//! outcomes into block/wake events. Sync state machines do not
//! themselves decide scheduling order; this type stays free of any
//! scheduler awareness.

/// A stable identifier for a signal notification register in the runtime.
///
/// In the Cell model a signal notification register is a small word
/// other units can OR-write into. The register state lives in
/// [`SignalRegister`]; this id is just the handle. There
/// is no `From<u64>` impl: id construction stays at registry sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SignalId(u64);

impl SignalId {
    /// Construct a `SignalId` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying id value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A signal notification register.
///
/// Holds a single 32-bit word. Updates are OR-merged: `or_in(v)` sets
/// the register to `current | v`. Idempotent under repeated identical
/// updates, monotonic in the bits-set sense, and trivially
/// deterministic. Clear semantics are explicit: `clear` resets the
/// register to zero, and there is no clear-on-read rule.
///
/// `SignalRegister` owns the value and nothing else: no `SignalId`
/// (the registry that owns the register knows its id), no event-queue
/// handle, no waiter list. Those are integration-layer concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SignalRegister {
    value: u32,
}

impl SignalRegister {
    /// Construct an empty register (value 0).
    #[inline]
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// Construct a register pre-loaded with `value`. Useful for
    /// scenario fixtures that need a non-zero initial state.
    #[inline]
    pub const fn with_value(value: u32) -> Self {
        Self { value }
    }

    /// Read the current register value.
    #[inline]
    pub const fn value(self) -> u32 {
        self.value
    }

    /// OR `bits` into the register. Returns the post-update value so
    /// callers can observe whether any new bits were actually set.
    #[inline]
    pub fn or_in(&mut self, bits: u32) -> u32 {
        self.value |= bits;
        self.value
    }

    /// Reset the register to zero. There is no clear-on-read rule;
    /// tests and the integration layer invoke this explicitly when
    /// they need to reset.
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
    fn default_is_zero() {
        assert_eq!(SignalId::default(), SignalId::new(0));
    }

    #[test]
    fn ordering_is_total() {
        assert!(SignalId::new(3) < SignalId::new(4));
        assert_eq!(SignalId::new(11), SignalId::new(11));
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
        // Each call must produce a value with at least as many bits
        // set as before. This monotonicity property is what makes
        // OR-merge safe to apply at commit time without ordering
        // between concurrent OR sources.
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
        // Order of independent OR-updates does not matter -- the
        // final value is the bitwise union either way. This is the
        // property that justifies "the runtime applies the OR at
        // commit time" without pinning per-source ordering.
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
