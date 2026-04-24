//! Distinct numeric types for the runtime's three time-like quantities.
//!
//! Guest ticks order guest-visible events, budgets bound per-step progress,
//! and epochs number commit batches. The three are separate types with no
//! implicit conversions so guest time can never silently become wall time
//! or scheduler currency. Scheduler policy lives elsewhere.

pub mod budget;
pub mod epoch;
pub mod ticks;

pub use budget::Budget;
pub use epoch::Epoch;
pub use ticks::GuestTicks;
