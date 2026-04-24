//! Stable newtype identifiers for units, events, and sequence numbers.

/// A stable identifier for an execution unit.
///
/// Distinct from [`EventId`], [`SequenceNumber`], and raw `u64`.
/// Assigned by the `cellgov_core` unit registry; two live units must never
/// share an id within a single runtime instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct UnitId(u64);

impl UnitId {
    /// Construct a `UnitId` from a raw value.
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

/// A stable identifier for an event in the runtime's event log.
///
/// Distinct from [`UnitId`], [`SequenceNumber`], and raw `u64`. Not part
/// of [`crate::OrderingKey`]; used only for trace correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EventId(u64);

impl EventId {
    /// Construct an `EventId` from a raw value.
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

/// Final tie-break counter for [`crate::OrderingKey`].
///
/// Distinct from [`UnitId`], [`EventId`], and raw `u64`. Overflow is an
/// invariant violation: [`SequenceNumber::next`] returns `None` at
/// `u64::MAX` rather than wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    /// The first sequence number issued by a fresh runtime.
    pub const ZERO: Self = Self(0);

    /// Construct a `SequenceNumber` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying counter value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The successor sequence number, or `None` on overflow.
    #[inline]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_id_roundtrip() {
        assert_eq!(UnitId::new(7).raw(), 7);
        assert_eq!(UnitId::default(), UnitId::new(0));
    }

    #[test]
    fn unit_id_ordering_is_total() {
        assert!(UnitId::new(1) < UnitId::new(2));
        assert_eq!(UnitId::new(5), UnitId::new(5));
    }

    #[test]
    fn event_id_roundtrip() {
        assert_eq!(EventId::new(42).raw(), 42);
        assert_eq!(EventId::default(), EventId::new(0));
    }

    #[test]
    fn event_id_ordering_is_total() {
        assert!(EventId::new(10) < EventId::new(11));
    }

    #[test]
    fn sequence_zero_is_origin() {
        assert_eq!(SequenceNumber::ZERO, SequenceNumber::new(0));
        assert_eq!(SequenceNumber::default(), SequenceNumber::ZERO);
    }

    #[test]
    fn sequence_next_advances_by_one() {
        assert_eq!(SequenceNumber::ZERO.next(), Some(SequenceNumber::new(1)));
        assert_eq!(
            SequenceNumber::new(99).next(),
            Some(SequenceNumber::new(100))
        );
    }

    #[test]
    fn sequence_next_at_max_is_none() {
        assert_eq!(SequenceNumber::new(u64::MAX).next(), None);
    }

    #[test]
    fn sequence_ordering_is_total_and_monotonic() {
        assert!(SequenceNumber::new(1) < SequenceNumber::new(2));
    }
}
