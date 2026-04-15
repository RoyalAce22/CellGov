//! Event priority class -- the second tier of the global ordering key.
//!
//! `PriorityClass` is a small total enum, not a numeric newtype: the
//! ordering rule lives in the discriminants directly so a misordered
//! priority is a compile-time fact. The variant set is fixed and the
//! relative ordering is part of
//! the runtime's determinism contract: a discriminant reorder would
//! silently change every tie-break in every replay. The variants and their
//! `#[repr(u8)]` discriminants are therefore locked.

/// Coarse priority class for events flowing through the runtime.
///
/// Lower discriminants order earlier under the derived `Ord`. The variant
/// order below is part of the determinism contract: do not reorder, do
/// not insert variants in the middle, do not change the explicit
/// discriminant values. New classes, if ever needed, must be appended
/// at the end and given an explicit discriminant strictly greater than
/// `Critical`.
///
/// The set is kept small: the minimum needed to let the scheduler
/// distinguish ordinary work from latency-sensitive wakeups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum PriorityClass {
    /// Best-effort work that may be deferred behind anything else.
    Background = 0,
    /// The default class. Most events should use this.
    #[default]
    Normal = 1,
    /// Latency-sensitive but not critical.
    High = 2,
    /// Reserved for runtime-essential events that must not be reordered
    /// behind any other class within the same timestamp.
    Critical = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_order_is_locked() {
        assert!(PriorityClass::Background < PriorityClass::Normal);
        assert!(PriorityClass::Normal < PriorityClass::High);
        assert!(PriorityClass::High < PriorityClass::Critical);
    }

    #[test]
    fn discriminants_are_locked() {
        assert_eq!(PriorityClass::Background as u8, 0);
        assert_eq!(PriorityClass::Normal as u8, 1);
        assert_eq!(PriorityClass::High as u8, 2);
        assert_eq!(PriorityClass::Critical as u8, 3);
    }

    #[test]
    fn default_is_normal() {
        assert_eq!(PriorityClass::default(), PriorityClass::Normal);
    }

    #[test]
    fn equality_is_reflexive() {
        assert_eq!(PriorityClass::High, PriorityClass::High);
        assert_ne!(PriorityClass::High, PriorityClass::Critical);
    }
}
