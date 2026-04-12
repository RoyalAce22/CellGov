//! The global ordering key for every guest-visible event in the runtime.
//!
//! Two events are compared by [`OrderingKey`], which encodes the
//! deterministic tie-break:
//!
//! 1. event timestamp ([`GuestTicks`])
//! 2. event priority class ([`PriorityClass`])
//! 3. source unit id ([`UnitId`])
//! 4. event sequence number ([`SequenceNumber`])
//!
//! No host time inputs. No reliance on `HashMap` iteration order. The
//! runtime stores events in stable ordered collections keyed by this type.

use crate::ids::{SequenceNumber, UnitId};
use crate::priority::PriorityClass;
use cellgov_time::GuestTicks;

/// The global ordering key shared by every guest-visible event.
///
/// **The field declaration order is load-bearing.** Rust's derived `Ord`
/// on a struct compares fields lexicographically top-to-bottom, and that
/// is exactly the required tie-break order. Reordering these fields would
/// silently change the ordering of every event in every replay. Do not
/// reorder them, do not insert fields in the middle, and do not change the
/// derive list to a hand-written `Ord` without re-establishing the same
/// invariant in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OrderingKey {
    /// Tier 1: guest-time stamp at which the event becomes visible.
    pub timestamp: GuestTicks,
    /// Tier 2: priority class. Lower discriminant orders earlier.
    pub priority: PriorityClass,
    /// Tier 3: source unit id. Stable across runs of a given scenario.
    pub source: UnitId,
    /// Tier 4: per-runtime monotonic sequence number issued at event
    /// creation time. Final tie-break -- guarantees a total order.
    pub sequence: SequenceNumber,
}

impl OrderingKey {
    /// Construct an ordering key from its four tiers.
    ///
    /// Provided as a convenience so call sites do not need to spell out
    /// the field names every time, but the struct fields remain `pub` so
    /// pattern matching and field updates still work.
    #[inline]
    pub const fn new(
        timestamp: GuestTicks,
        priority: PriorityClass,
        source: UnitId,
        sequence: SequenceNumber,
    ) -> Self {
        Self {
            timestamp,
            priority,
            source,
            sequence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: u64, p: PriorityClass, u: u64, s: u64) -> OrderingKey {
        OrderingKey::new(
            GuestTicks::new(t),
            p,
            UnitId::new(u),
            SequenceNumber::new(s),
        )
    }

    #[test]
    fn tier_one_timestamp_dominates_everything() {
        // Earlier timestamp wins regardless of higher priority, larger
        // source id, or larger sequence number.
        let earlier = key(10, PriorityClass::Background, 999, 999);
        let later = key(11, PriorityClass::Critical, 0, 0);
        assert!(earlier < later);
    }

    #[test]
    fn tier_two_priority_breaks_timestamp_tie() {
        // Same timestamp -- lower priority class wins.
        let bg = key(50, PriorityClass::Background, 999, 999);
        let normal = key(50, PriorityClass::Normal, 0, 0);
        let high = key(50, PriorityClass::High, 0, 0);
        let crit = key(50, PriorityClass::Critical, 0, 0);
        assert!(bg < normal);
        assert!(normal < high);
        assert!(high < crit);
    }

    #[test]
    fn tier_three_source_breaks_priority_tie() {
        // Same timestamp and priority -- lower source unit id wins.
        let lo = key(50, PriorityClass::Normal, 1, 999);
        let hi = key(50, PriorityClass::Normal, 2, 0);
        assert!(lo < hi);
    }

    #[test]
    fn tier_four_sequence_breaks_source_tie() {
        // Everything else equal -- lower sequence number wins.
        let lo = key(50, PriorityClass::Normal, 7, 1);
        let hi = key(50, PriorityClass::Normal, 7, 2);
        assert!(lo < hi);
    }

    #[test]
    fn equal_keys_compare_equal() {
        let a = key(50, PriorityClass::High, 3, 9);
        let b = key(50, PriorityClass::High, 3, 9);
        assert_eq!(a, b);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    }

    #[test]
    fn default_is_lowest_possible_key() {
        let d = OrderingKey::default();
        assert_eq!(d.timestamp, GuestTicks::ZERO);
        assert_eq!(d.priority, PriorityClass::default());
        assert_eq!(d.source, UnitId::new(0));
        assert_eq!(d.sequence, SequenceNumber::ZERO);
    }

    #[test]
    fn ordering_is_total_across_a_mixed_set() {
        // A small mixed set sorted via the derived Ord. The expected
        // order exercises every tier of the tie-break.
        let mut keys = [
            key(2, PriorityClass::Normal, 0, 0),
            key(1, PriorityClass::Critical, 99, 99),
            key(1, PriorityClass::Background, 5, 0),
            key(1, PriorityClass::Background, 5, 1),
            key(1, PriorityClass::Background, 4, 7),
            key(1, PriorityClass::Normal, 0, 0),
        ];
        keys.sort();
        let expected = [
            key(1, PriorityClass::Background, 4, 7),
            key(1, PriorityClass::Background, 5, 0),
            key(1, PriorityClass::Background, 5, 1),
            key(1, PriorityClass::Normal, 0, 0),
            key(1, PriorityClass::Critical, 99, 99),
            key(2, PriorityClass::Normal, 0, 0),
        ];
        assert_eq!(keys, expected);
    }
}
