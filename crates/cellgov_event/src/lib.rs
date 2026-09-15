//! Event identifiers, ordering key, and priority classes.
//!
//! Global ordering tie-break: timestamp, priority class, source unit,
//! sequence number.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod ids;
pub mod ordering;
pub mod priority;

pub use ids::{EventId, SequenceNumber, UnitId};
pub use ordering::OrderingKey;
pub use priority::PriorityClass;
