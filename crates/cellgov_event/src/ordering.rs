//! Global ordering key for guest-visible events.

use crate::ids::{SequenceNumber, UnitId};
use crate::priority::PriorityClass;
use cellgov_time::GuestTicks;

/// Total order over guest-visible events.
///
/// Field declaration order IS the tie-break order: the derived `Ord`
/// compares lexicographically top-to-bottom. Reordering fields changes
/// every replay's event order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OrderingKey {
    /// Guest-time stamp at which the event becomes visible.
    pub timestamp: GuestTicks,
    /// Lower discriminant orders earlier.
    pub priority: PriorityClass,
    /// Source unit id.
    pub source: UnitId,
    /// Per-runtime monotonic counter; final tie-break guaranteeing totality.
    pub sequence: SequenceNumber,
}

impl OrderingKey {
    /// Construct an ordering key from its four tiers.
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
        let earlier = key(10, PriorityClass::Background, 999, 999);
        let later = key(11, PriorityClass::Critical, 0, 0);
        assert!(earlier < later);
    }

    #[test]
    fn tier_two_priority_breaks_timestamp_tie() {
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
        let lo = key(50, PriorityClass::Normal, 1, 999);
        let hi = key(50, PriorityClass::Normal, 2, 0);
        assert!(lo < hi);
    }

    #[test]
    fn tier_four_sequence_breaks_source_tie() {
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
