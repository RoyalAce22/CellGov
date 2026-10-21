//! Guest ticks converted to the time-base register, to seconds and
//! nanoseconds, and from microseconds, nanoseconds and SPU cycles.
//!
//! A guest tick is one cycle of the 3.192 GHz core clock. The time
//! base, an SPU cycle and a microsecond are each a whole number of
//! ticks, so the conversion from each of them to ticks is exact. The
//! conversion from ticks to the time base rounds down. A nanosecond is
//! 399/125 of a tick, so a conversion to or from nanoseconds rounds
//! down.

use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
use cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK;

/// Guest ticks per time-base tick: the core clock, which is the SPU
/// clock, runs at 40 times the time-base frequency.
pub const TICKS_PER_TB_TICK: u64 = SPU_CYCLES_PER_TB_TICK;

/// Guest ticks per simulated wall-clock second: the 3.192 GHz core
/// clock.
pub const TICKS_PER_SECOND: u64 = TICKS_PER_TB_TICK * CELL_PPU_TIMEBASE_HZ;

/// Guest ticks per simulated microsecond.
pub const TICKS_PER_USEC: u64 = TICKS_PER_SECOND / 1_000_000;

const _: () = assert!(TICKS_PER_SECOND == 3_192_000_000);
const _: () = assert!(TICKS_PER_USEC * 1_000_000 == TICKS_PER_SECOND);

/// Nanoseconds per tick, as a fraction: 10^9 / 3.192*10^9 = 125/399.
const NSEC_NUM: u64 = 125;
const NSEC_DEN: u64 = 399;

const _: () = assert!(TICKS_PER_SECOND / NSEC_DEN * NSEC_NUM == 1_000_000_000);

/// Convert a guest-tick count to the TB register value a coherent
/// `mftb` read would return. Rounds down.
#[inline]
pub const fn ticks_to_tb(ticks: u64) -> u64 {
    ticks / TICKS_PER_TB_TICK
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
    let num = tb_num as u128 * TICKS_PER_TB_TICK as u128;
    let ticks = num.div_ceil(tb_den as u128);
    if ticks > u64::MAX as u128 {
        u64::MAX
    } else {
        ticks as u64
    }
}

/// The guest ticks that `cycles` SPU clock cycles take.
///
/// The SPU clock is the core clock, so one cycle is one tick.
#[inline]
pub const fn spu_cycles_to_ticks(cycles: u64) -> u64 {
    cycles
}

/// The guest ticks that `nsec` nanoseconds take, rounded down. Saturates
/// at the tick ceiling.
///
/// A caller that charges time in nanoseconds converts a running total
/// from one fixed start. Each read then includes the partial tick that
/// the previous read dropped.
#[inline]
pub const fn nsec_to_ticks(nsec: u64) -> u64 {
    let ticks = (nsec as u128 * NSEC_DEN as u128) / NSEC_NUM as u128;
    if ticks > u64::MAX as u128 {
        u64::MAX
    } else {
        ticks as u64
    }
}

/// The guest ticks that `usec` microseconds take. Saturates at the tick
/// ceiling.
#[inline]
pub const fn usec_to_ticks(usec: u64) -> u64 {
    usec.saturating_mul(TICKS_PER_USEC)
}

/// Convert a guest-tick count to whole nanoseconds, rounded down.
#[inline]
pub const fn ticks_to_nsec(ticks: u64) -> u64 {
    ((ticks as u128 * NSEC_NUM as u128) / NSEC_DEN as u128) as u64
}

/// Convert a guest-tick count to the `(sec, nsec)` pair
/// `sys_time_get_current_time` writes through its out-pointers.
///
/// `nsec` rounds down and is always in `0..=999_999_999`.
#[inline]
pub const fn ticks_to_sec_nsec(ticks: u64) -> (u64, u64) {
    let sec = ticks / TICKS_PER_SECOND;
    let nsec = ticks_to_nsec(ticks % TICKS_PER_SECOND);
    (sec, nsec)
}

#[cfg(test)]
#[path = "tests/conversion_tests.rs"]
mod tests;
