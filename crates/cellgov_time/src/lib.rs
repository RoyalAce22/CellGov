//! Distinct numeric types for the runtime's three time-like quantities.
//!
//! Guest ticks order guest-visible events, budgets bound per-step progress,
//! and epochs number commit batches. The three are separate types with no
//! implicit conversions so guest time can never silently become wall time
//! or scheduler currency. Scheduler policy lives elsewhere.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod budget;
mod conversion;
pub mod epoch;
pub mod ticks;

pub use budget::{Budget, Consume, InstructionCost};
pub use conversion::{ticks_to_sec_nsec, ticks_to_tb, SIMULATED_INSTRUCTIONS_PER_SECOND};
pub use epoch::Epoch;
pub use ticks::GuestTicks;

pub use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
