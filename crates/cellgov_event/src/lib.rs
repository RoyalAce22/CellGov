//! cellgov_event -- event identifiers, ordering logic, priority classes.
//!
//! Owns `UnitId`, `EventId`, `SequenceNumber`, `OrderingKey`,
//! `PriorityClass`, and the deterministic tie-break metadata used by the
//! scheduler.
//!
//! Global ordering key (do not deviate):
//!
//! 1. event timestamp
//! 2. event priority class
//! 3. source unit id
//! 4. event sequence number
//!
//! Do not rely on insertion order from `HashMap` or host thread timing. Use
//! stable ordered collections where determinism matters.

pub mod ids;
pub mod ordering;
pub mod priority;

pub use ids::{EventId, SequenceNumber, UnitId};
pub use ordering::OrderingKey;
pub use priority::PriorityClass;
