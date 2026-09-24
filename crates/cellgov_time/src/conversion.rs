//! Guest ticks converted to the timebase register and to seconds and
//! nanoseconds.

use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;

/// Simulated rate at which the interpreted PPU "runs", in instructions
/// per simulated wall-clock second.
///
/// `GuestTicks` increments once per retired instruction. 10^9 ticks/s
/// makes 1 tick = 1 nanosecond of simulated time, which keeps the
/// tick-to-(sec,nsec) conversion integer-exact. Not a cycle-accurate
/// model; the interpreter does not simulate IPC.
pub const SIMULATED_INSTRUCTIONS_PER_SECOND: u64 = 1_000_000_000;

/// Convert a guest-tick count to the TB register value a coherent
/// `mftb` read would return.
///
/// Uses u128 arithmetic so the multiplication does not overflow for
/// any reachable `ticks` value. `ticks * TB_HZ / SIM_IPS`.
#[inline]
pub const fn ticks_to_tb(ticks: u64) -> u64 {
    ((ticks as u128 * CELL_PPU_TIMEBASE_HZ as u128) / SIMULATED_INSTRUCTIONS_PER_SECOND as u128)
        as u64
}

/// Convert a guest-tick count to the `(sec, nsec)` pair
/// `sys_time_get_current_time` writes through its out-pointers.
///
/// `nsec` is always in `0..=999_999_999`.
#[inline]
pub const fn ticks_to_sec_nsec(ticks: u64) -> (u64, u64) {
    let sec = ticks / SIMULATED_INSTRUCTIONS_PER_SECOND;
    let nsec = ticks % SIMULATED_INSTRUCTIONS_PER_SECOND;
    (sec, nsec)
}

#[cfg(test)]
#[path = "tests/conversion_tests.rs"]
mod tests;
