//! cellgov_time -- guest virtual time, progress budgets, timestamp arithmetic.
//!
//! This crate owns the runtime's notion of time. Guest time is the authoritative
//! ordering clock for all guest-visible events. It is monotonic across the entire
//! runtime, not per-unit.
//!
//! Three concepts live here and must remain distinct types -- never plain `u64`,
//! never implicitly convertible:
//!
//! - guest ticks: the authoritative ordering clock
//! - budget units: a policy input granted to a unit at scheduling time
//! - logical epoch: advances at commit boundaries, granularity for state hashes
//!
//! No scheduler policy lives in this crate.

pub mod budget;
pub mod epoch;
pub mod ticks;

pub use budget::Budget;
pub use epoch::Epoch;
pub use ticks::GuestTicks;
