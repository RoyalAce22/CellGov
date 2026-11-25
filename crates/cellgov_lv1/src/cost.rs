//! The guest time a hypercall charges.
//!
//! Each cost is the console's round trip for the call less the cycles of
//! the kernel's own path to it. No capture measures one yet, so every
//! number is untimed.

use cellgov_time::GuestTicks;

/// The cost of each timed number, in guest ticks, ascending by number.
const COSTS: &[(u64, u64)] = &[];

/// The guest ticks hypercall `number` charges, or `None` when the number
/// is untimed.
pub fn cost(number: u64) -> Option<GuestTicks> {
    COSTS
        .iter()
        .find(|(n, _)| *n == number)
        .map(|(_, ticks)| GuestTicks::new(*ticks))
}
