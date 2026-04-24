//! Priority class enum -- second tier of [`crate::OrderingKey`].

/// Coarse priority class for events flowing through the runtime.
///
/// Variant order and `#[repr(u8)]` discriminants are part of the
/// determinism contract: a reorder silently changes every tie-break in
/// every replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum PriorityClass {
    /// Best-effort work that may be deferred behind anything else.
    Background = 0,
    /// The default class.
    #[default]
    Normal = 1,
    /// Latency-sensitive but not critical.
    High = 2,
    /// Runtime-essential events that must not be reordered behind any
    /// other class within the same timestamp.
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
