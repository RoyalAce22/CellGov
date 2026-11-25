//! The guest time a hypercall charges.
//!
//! Each cost is the console's round trip for the call less the cycles of
//! the kernel's own path to it. Each registration carries its own cost.

use cellgov_time::GuestTicks;

use super::group::table;

/// The guest ticks hypercall `number` charges, or `None` when the number
/// is untimed or no group answers it.
pub fn cost(number: u64) -> Option<GuestTicks> {
    table::registration(number).and_then(|hypercall| hypercall.cost)
}
