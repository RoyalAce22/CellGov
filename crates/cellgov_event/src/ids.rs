//! Stable newtype identifiers for units, events, and sequence numbers.

/// A stable identifier for an execution unit.
///
/// Distinct from [`EventId`], [`SequenceNumber`], and raw `u64`.
/// Assigned by the `cellgov_core` unit registry; two live units must
/// never share an id within a single runtime instance.
///
/// Ordering is arbitrary-but-stable; `a < b` does not imply `a` was
/// registered first or is otherwise temporally prior. Provided so
/// `BTreeMap<UnitId, _>` works for deterministic per-unit storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(u64);

impl UnitId {
    /// Construct from a raw value. Production code receives ids from
    /// the registry; `new` is the trace-replay and scenario path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: state hashing
    /// (`registry::runnable_hash`, `lv2_host::content_hash`),
    /// diagnostic printing, and cross-domain id translation
    /// (`UnitId` to `MailboxId` in dispatch paths).
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A stable identifier for an event in the runtime's event log.
///
/// Distinct from [`UnitId`], [`SequenceNumber`], and raw `u64`. Not
/// part of [`crate::OrderingKey`]; used only for trace correlation.
///
/// Ordering is arbitrary-but-stable; `a < b` does not imply event
/// `a` happened before event `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

impl EventId {
    /// Construct from a raw value; trace-replay and scenario path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: trace serialization and
    /// diagnostic printing.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Final tie-break counter for [`crate::OrderingKey`].
///
/// Distinct from [`UnitId`], [`EventId`], and raw `u64`. Overflow is
/// an invariant violation: [`SequenceNumber::next`] returns `None`
/// at `u64::MAX` rather than wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    /// The first sequence number issued by a fresh runtime.
    pub const ZERO: Self = Self(0);

    /// Construct from a raw value; trace-replay path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying counter value. Consumers: trace wire format,
    /// `OrderingKey` packing, and diagnostic printing.
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
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn unit_id_roundtrip() {
        assert_eq!(UnitId::new(7).raw(), 7);
    }

    #[test]
    fn unit_id_ordering_is_total() {
        assert!(UnitId::new(1) < UnitId::new(2));
        assert_eq!(UnitId::new(5), UnitId::new(5));
    }

    #[test]
    fn unit_id_hash_matches_eq() {
        assert_eq!(hash(&UnitId::new(7)), hash(&UnitId::new(7)));
        assert_ne!(hash(&UnitId::new(7)), hash(&UnitId::new(8)));
    }

    #[test]
    fn unit_id_copy_preserves_value() {
        let a = UnitId::new(42);
        let b = a;
        assert_eq!(a, b);
        assert_eq!(a.raw(), 42);
        assert_eq!(hash(&a), hash(&b));
    }

    #[test]
    fn event_id_roundtrip() {
        assert_eq!(EventId::new(42).raw(), 42);
    }

    #[test]
    fn event_id_ordering_is_total() {
        assert!(EventId::new(10) < EventId::new(11));
    }

    #[test]
    fn event_id_hash_matches_eq() {
        assert_eq!(hash(&EventId::new(42)), hash(&EventId::new(42)));
        assert_ne!(hash(&EventId::new(42)), hash(&EventId::new(43)));
    }

    /// Derived `Hash` on a single-field newtype delegates to the
    /// inner `u64`, so wrappers with the same raw value DO hash
    /// identically. The wall against `UnitId`/`EventId` confusion
    /// lives at the type level, not the hash level.
    #[test]
    fn unit_and_event_ids_share_hash_when_raw_collides() {
        assert_eq!(hash(&UnitId::new(7)), hash(&EventId::new(7)));
        assert_eq!(hash(&UnitId::new(7).raw()), hash(&EventId::new(7).raw()));
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
    fn sequence_chain_from_zero_is_monotonic() {
        let s0 = SequenceNumber::ZERO;
        let s1 = s0.next().unwrap();
        let s2 = s1.next().unwrap();
        let s3 = s2.next().unwrap();
        assert_eq!(s3, SequenceNumber::new(3));
        assert!(s0 < s1 && s1 < s2 && s2 < s3);
    }

    #[test]
    fn sequence_next_at_max_is_none() {
        assert_eq!(SequenceNumber::new(u64::MAX).next(), None);
    }

    #[test]
    fn sequence_ordering_is_total_and_monotonic() {
        assert!(SequenceNumber::new(1) < SequenceNumber::new(2));
    }

    #[test]
    fn sequence_hash_matches_eq() {
        assert_eq!(hash(&SequenceNumber::new(5)), hash(&SequenceNumber::new(5)));
        assert_ne!(hash(&SequenceNumber::new(5)), hash(&SequenceNumber::new(6)));
    }
}
