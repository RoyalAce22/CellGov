//! Event identifiers, ordering key, and priority classes.
//!
//! Global ordering tie-break: timestamp, priority class, source unit,
//! sequence number.

pub mod ids;
pub mod ordering;
pub mod priority;

pub use ids::{EventId, SequenceNumber, UnitId};
pub use ordering::OrderingKey;
pub use priority::PriorityClass;
