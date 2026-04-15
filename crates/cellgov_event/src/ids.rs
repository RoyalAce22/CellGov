//! Stable identifiers used by the event layer.
//!
//! Three newtypes live here: [`UnitId`], [`EventId`], and [`SequenceNumber`].
//! They are kept distinct from each other and from the time types in
//! [`cellgov_time`]: a tick is not a sequence number, a unit is not an event,
//! and none of them are interchangeable with a bare `u64`. The runtime's
//! determinism contract requires every id construction site to be visible,
//! so there are no `From<u64>` impls -- every lift uses [`UnitId::new`] and
//! friends.
//!
//! `UnitId` is defined here, not in `cellgov_core`, because the global
//! ordering key in `cellgov_event` must mention it and `cellgov_event` sits
//! below `cellgov_core` in the workspace dependency DAG. The unit registry
//! in `cellgov_core` hands `UnitId` instances out at unit construction
//! time; the type itself is plain data.

/// A stable identifier for an execution unit.
///
/// `UnitId`s are assigned by the unit registry in `cellgov_core` and are
/// recorded in the trace at construction time. They participate in the
/// global ordering key as the third tier of the deterministic tie-break
/// (after timestamp and priority class), so two units must never share an
/// id within a single runtime instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct UnitId(u64);

impl UnitId {
    /// Construct a `UnitId` from a raw value.
    ///
    /// There is no `From<u64>` impl: id assignment is the registry's job,
    /// and ad-hoc construction outside the registry should be visible at
    /// the call site.
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
/// `EventId`s are issued at event creation time and used for trace
/// correlation and replay assertions. They are not part of the ordering
/// key: ordering uses [`crate::OrderingKey`], which carries
/// timestamp, priority, source unit, and sequence number. Event ids only
/// disambiguate two distinct events in trace records.
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

/// A monotonically increasing tie-break counter.
///
/// `SequenceNumber` is the fourth and final tier of the global ordering
/// key. When timestamp, priority class, and source unit id all match
/// between two events, the sequence number breaks the tie. It is issued
/// at event creation time by a runtime-owned counter and is therefore
/// unique within a single runtime instance.
///
/// Like [`cellgov_time::Epoch`], overflow is an invariant violation rather
/// than wraparound: [`SequenceNumber::next`] returns `None` at `u64::MAX`
/// instead of silently rolling over and producing duplicate ordering keys.
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
