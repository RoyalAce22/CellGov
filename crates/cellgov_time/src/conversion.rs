//! Guest ticks converted to the time-base register and to seconds and
//! nanoseconds, and SPU clock cycles converted to guest ticks.

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

/// The least guest-tick count whose time base covers `tb_num / tb_den`
/// time-base ticks: the inverse of [`ticks_to_tb`], rounded up.
///
/// `tb_den` of 0 answers `u64::MAX`, as does a count past the tick
/// range.
#[inline]
pub const fn tb_to_ticks(tb_num: u64, tb_den: u64) -> u64 {
    if tb_den == 0 {
        return u64::MAX;
    }
    let num = tb_num as u128 * SIMULATED_INSTRUCTIONS_PER_SECOND as u128;
    let den = tb_den as u128 * CELL_PPU_TIMEBASE_HZ as u128;
    let ticks = num.div_ceil(den);
    if ticks > u64::MAX as u128 {
        u64::MAX
    } else {
        ticks as u64
    }
}

/// The guest ticks that `cycles` SPU clock cycles take, rounded down.
///
/// The SPU clock runs at [`SPU_CYCLES_PER_TB_TICK`] times the time-base
/// frequency.
///
/// The caller converts a running cycle total from one fixed start. Each
/// read then includes the partial tick that the previous read dropped.
///
/// [`SPU_CYCLES_PER_TB_TICK`]: cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK
#[inline]
pub const fn spu_cycles_to_ticks(cycles: u64) -> u64 {
    let den = cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK as u128
        * CELL_PPU_TIMEBASE_HZ as u128;
    ((cycles as u128 * SIMULATED_INSTRUCTIONS_PER_SECOND as u128) / den) as u64
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
