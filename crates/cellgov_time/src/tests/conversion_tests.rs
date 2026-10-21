//! The tick conversions: exact for the time base, SPU cycles and
//! microseconds, rounded down to and from nanoseconds.

use super::*;

#[test]
fn a_second_of_ticks_is_a_second_of_time_base() {
    assert_eq!(ticks_to_tb(0), 0);
    assert_eq!(ticks_to_tb(TICKS_PER_SECOND), CELL_PPU_TIMEBASE_HZ);
    assert_eq!(
        ticks_to_tb(1_000 * TICKS_PER_SECOND),
        1_000 * CELL_PPU_TIMEBASE_HZ
    );
}

#[test]
fn ticks_to_tb_rounds_down_within_a_time_base_tick() {
    assert_eq!(ticks_to_tb(39), 0);
    assert_eq!(ticks_to_tb(40), 1);
    assert_eq!(ticks_to_tb(79), 1);
}

#[test]
fn tb_to_ticks_is_the_least_tick_that_reaches_the_time_base() {
    for tb in [0u64, 1, 2, 79, 80, 50_001, 79_800_000] {
        let ticks = tb_to_ticks(tb, 1);
        assert_eq!(ticks, 40 * tb, "tb {tb}: a whole time-base tick is exact");
        assert!(ticks_to_tb(ticks) >= tb, "tb {tb}");
        if ticks > 0 {
            assert!(ticks_to_tb(ticks - 1) < tb, "tb {tb}: the least such tick");
        }
    }
    assert_eq!(tb_to_ticks(CELL_PPU_TIMEBASE_HZ, 1), TICKS_PER_SECOND);
}

#[test]
fn tb_to_ticks_rounds_a_fraction_up_and_refuses_a_zero_denominator() {
    // 386 time-base ticks over 32 runs: 482.5 ticks.
    assert_eq!(tb_to_ticks(386, 32), 483);
    // 384 over 32 is 480 exactly.
    assert_eq!(tb_to_ticks(384, 32), 480);
    assert_eq!(tb_to_ticks(1, 0), u64::MAX);
    assert_eq!(tb_to_ticks(u64::MAX, 1), u64::MAX);
}

#[test]
fn an_spu_cycle_is_one_tick() {
    for cycles in [0u64, 1, 39, 40, 399, 140_928_623] {
        assert_eq!(spu_cycles_to_ticks(cycles), cycles);
    }
    assert_eq!(ticks_to_tb(spu_cycles_to_ticks(40 * 50_000)), 50_000);
}

#[test]
fn a_microsecond_is_3192_ticks_and_saturates() {
    assert_eq!(TICKS_PER_USEC, 3_192);
    assert_eq!(usec_to_ticks(0), 0);
    assert_eq!(usec_to_ticks(1), 3_192);
    assert_eq!(usec_to_ticks(1_000_000), TICKS_PER_SECOND);
    assert_eq!(usec_to_ticks(u64::MAX), u64::MAX);
}

#[test]
fn ticks_to_nsec_rounds_down_and_is_exact_at_399_ticks() {
    // 399 ticks are 125 ns exactly.
    assert_eq!(ticks_to_nsec(399), 125);
    assert_eq!(ticks_to_nsec(398), 124);
    assert_eq!(ticks_to_nsec(3), 0);
    assert_eq!(ticks_to_nsec(4), 1);
    assert_eq!(ticks_to_nsec(TICKS_PER_SECOND), 1_000_000_000);
    // No overflow at the top of the range.
    assert_eq!(
        ticks_to_nsec(u64::MAX),
        (u64::MAX as u128 * 125 / 399) as u64
    );
}

#[test]
fn ticks_to_nsec_never_decreases() {
    let mut last = 0;
    for ticks in 0..2_000u64 {
        let ns = ticks_to_nsec(ticks);
        assert!(ns >= last, "tick {ticks}");
        last = ns;
    }
}

#[test]
fn ticks_to_sec_nsec_splits_at_a_second_of_ticks() {
    assert_eq!(ticks_to_sec_nsec(0), (0, 0));
    assert_eq!(ticks_to_sec_nsec(4), (0, 1));
    assert_eq!(ticks_to_sec_nsec(TICKS_PER_SECOND - 1), (0, 999_999_999));
    assert_eq!(ticks_to_sec_nsec(TICKS_PER_SECOND), (1, 0));
    assert_eq!(ticks_to_sec_nsec(TICKS_PER_SECOND + 4), (1, 1));
}

#[test]
fn ticks_to_sec_nsec_nsec_is_always_in_range() {
    for ticks in [0u64, 1, 1_500_000_000, 7_123_456_789, u64::MAX] {
        let (_sec, nsec) = ticks_to_sec_nsec(ticks);
        assert!(nsec < 1_000_000_000);
    }
}

#[test]
fn nsec_to_ticks_rounds_down_and_inverts_ticks_to_nsec() {
    assert_eq!(nsec_to_ticks(0), 0);
    assert_eq!(nsec_to_ticks(1), 3);
    assert_eq!(nsec_to_ticks(125), 399);
    assert_eq!(nsec_to_ticks(1_000), 3_192);
    assert_eq!(nsec_to_ticks(1_000_000_000), TICKS_PER_SECOND);
    assert_eq!(nsec_to_ticks(u64::MAX), u64::MAX);
    for nsec in [1u64, 2, 255, 256, 1_000_003] {
        assert_eq!(ticks_to_nsec(nsec_to_ticks(nsec) + 1), nsec);
    }
}
