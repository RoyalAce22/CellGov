//! Global ordering key for guest-visible events.

use crate::ids::{SequenceNumber, UnitId};
use crate::priority::PriorityClass;
use cellgov_time::GuestTicks;

/// Total order over guest-visible events.
///
/// Field declaration order IS the tie-break order; reordering
/// fields changes every replay. Lower keys sort first: a min-heap
/// or `BTreeMap` consumer pops the next event to service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderingKey {
    /// Guest-time stamp at which the event becomes visible.
    pub timestamp: GuestTicks,
    /// Higher priority sorts first; see [`PriorityClass`].
    pub priority: PriorityClass,
    /// Source unit id.
    pub source: UnitId,
    /// Per-runtime monotonic counter; final tie-break guaranteeing totality.
    pub sequence: SequenceNumber,
}

impl OrderingKey {
    /// Construct from the four tiers in declaration order.
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
#[path = "tests/ordering_tests.rs"]
mod tests;
