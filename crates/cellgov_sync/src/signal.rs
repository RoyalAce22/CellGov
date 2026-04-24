//! Signal notification register and its leaf identifier. Updates
//! OR-merge into the 32-bit value; clear is explicit, no
//! clear-on-read. Block/wake translation happens upstream.

/// Stable identifier for a signal notification register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SignalId(u64);

impl SignalId {
    /// Construct a `SignalId` from a raw value.
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

/// 32-bit OR-merge signal register. Idempotent, commutative,
/// monotonic-in-bits-set across repeated `or_in` calls.
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
