//! Tick-to-timebase and tick-to-sec/nsec conversion scaling without overflow.

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
