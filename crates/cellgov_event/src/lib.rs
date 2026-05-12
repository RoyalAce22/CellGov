//! Event identifiers, ordering key, and priority classes.
//!
//! Global ordering tie-break: timestamp, priority class, source unit,
//! sequence number.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod ids;
pub mod ordering;
pub mod priority;

pub use ids::{EventId, SequenceNumber, UnitId};
pub use ordering::OrderingKey;
pub use priority::PriorityClass;
