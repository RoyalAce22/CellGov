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
fn tb_to_ticks_is_the_least_tick_that_reaches_the_time_base() {
    for tb in [0u64, 1, 2, 79, 80, 50_001, 79_800_000] {
        let ticks = tb_to_ticks(tb, 1);
        assert!(ticks_to_tb(ticks) >= tb, "tb {tb}");
        if ticks > 0 {
            assert!(ticks_to_tb(ticks - 1) < tb, "tb {tb}: the least such tick");
        }
    }
    assert_eq!(
        tb_to_ticks(CELL_PPU_TIMEBASE_HZ, 1),
        SIMULATED_INSTRUCTIONS_PER_SECOND
    );
}

#[test]
fn tb_to_ticks_rounds_a_fraction_up_and_refuses_a_zero_denominator() {
    // 386 ticks over 32 runs: 12.0625 time-base ticks, 151.1 ns.
    assert_eq!(tb_to_ticks(386, 32), 152);
    assert_eq!(tb_to_ticks(1, 0), u64::MAX);
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
