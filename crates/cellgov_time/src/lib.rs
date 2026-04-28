//! Distinct numeric types for the runtime's three time-like quantities.
//!
//! Guest ticks order guest-visible events, budgets bound per-step progress,
//! and epochs number commit batches. The three are separate types with no
//! implicit conversions so guest time can never silently become wall time
//! or scheduler currency. Scheduler policy lives elsewhere.

pub mod budget;
pub mod epoch;
pub mod ticks;

pub use budget::{Budget, Consume, InstructionCost};
pub use epoch::Epoch;
pub use ticks::GuestTicks;

pub use cellgov_ps3_abi::hardware::CELL_PPU_TIMEBASE_HZ;

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
mod tests {
    use super::*;

    #[test]
    fn ticks_to_tb_is_zero_at_origin() {
        assert_eq!(ticks_to_tb(0), 0);
    }

    #[test]
    fn ticks_to_tb_scales_linearly() {
        // 1 sec of sim time = SIM_IPS ticks = TB_HZ TB ticks.
        assert_eq!(
            ticks_to_tb(SIMULATED_INSTRUCTIONS_PER_SECOND),
            CELL_PPU_TIMEBASE_HZ
        );
    }

    #[test]
    fn ticks_to_tb_handles_huge_counts_without_overflow() {
        // 1000 simulated seconds: tick count 10^12; raw multiplication
        // would exceed u64 but u128 intermediate is fine.
        let ticks = 1_000 * SIMULATED_INSTRUCTIONS_PER_SECOND;
        assert_eq!(ticks_to_tb(ticks), 1_000 * CELL_PPU_TIMEBASE_HZ);
    }

    #[test]
    fn ticks_to_sec_nsec_splits_at_billion() {
        assert_eq!(ticks_to_sec_nsec(0), (0, 0));
        assert_eq!(ticks_to_sec_nsec(1), (0, 1));
        assert_eq!(ticks_to_sec_nsec(999_999_999), (0, 999_999_999));
        assert_eq!(ticks_to_sec_nsec(1_000_000_000), (1, 0));
        assert_eq!(ticks_to_sec_nsec(1_000_000_001), (1, 1));
    }

    #[test]
    fn ticks_to_sec_nsec_nsec_is_always_in_range() {
        for ticks in [0u64, 1, 1_500_000_000, 7_123_456_789, u64::MAX] {
            let (_sec, nsec) = ticks_to_sec_nsec(ticks);
            assert!(nsec < SIMULATED_INSTRUCTIONS_PER_SECOND);
        }
    }
}
