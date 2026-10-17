//! The decrementer's count, its zero crossing, the stop, and the tick a
//! parked unit wakes at.

use super::*;

#[test]
fn a_new_decrementer_runs_down_from_the_top_from_its_first_look() {
    let mut d = Decrementer::new();
    assert_eq!(d.read(1_000), (u32::MAX, false), "the first look starts it");
    assert_eq!(d.read(1_250), (u32::MAX - 250, false));
}

#[test]
fn the_count_crosses_zero_exactly_when_more_ticks_elapse_than_it_held() {
    let mut d = Decrementer::new();
    d.write(0, 100);
    assert_eq!(d.read(100), (0, false), "at zero, not past it");
    assert_eq!(d.read(101), (u32::MAX, true), "one interval past zero");
    assert_eq!(
        d.read(102),
        (u32::MAX - 1, false),
        "the crossing counts once"
    );
}

#[test]
fn a_crossing_between_two_looks_is_seen_however_far_apart_they_are() {
    let mut d = Decrementer::new();
    d.write(0, 5);
    let (_, crossed) = d.read(5 + (1 << 33));
    assert!(crossed);
}

#[test]
fn a_load_that_sets_the_top_bit_signals_at_once_and_a_load_of_zero_after_one_interval() {
    let mut d = Decrementer::new();
    d.write(0, 10);
    assert!(d.write(1, 0x8000_0000), "0 to 1 in the top bit");
    assert!(!d.write(1, 0x8000_0001), "already 1: no change");
    assert!(!d.write(3, 0));
    assert_eq!(d.read(4), (u32::MAX, true));
}

#[test]
fn a_stopped_count_holds_and_a_write_starts_it_again() {
    let mut d = Decrementer::new();
    d.write(0, 1_000);
    assert!(!d.stop(400));
    assert_eq!(d.read(10_000), (600, false));
    assert!(!d.running());
    assert!(!d.write(10_000, 0x7FFF_FFFF));
    assert_eq!(d.read(10_010), (0x7FFF_FFFF - 10, false));
}

#[test]
fn the_crossing_tick_is_the_first_guest_tick_past_zero() {
    let mut d = Decrementer::new();
    let start = GuestTicks::new(12_345);
    d.write(time_base_at(start), 50_000);
    let tick = d.crossing_tick().expect("running");
    let tb_start = time_base_at(start);
    assert_eq!(time_base_at(tick) - tb_start, 50_001);
    assert_eq!(
        time_base_at(GuestTicks::new(tick.raw() - 1)) - tb_start,
        50_000
    );
    d.stop(time_base_at(start));
    assert_eq!(d.crossing_tick(), None);
}
