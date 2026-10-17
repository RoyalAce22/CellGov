//! The timebase frequency held to the `lv2_timer_time_tty` capture from
//! the reference console: the word `sys_time_get_timebase_frequency`
//! (147) returns, and the timebase ticks a one-second
//! `sys_timer_sleep` (142) took.

use cellgov_event::UnitId;

use super::*;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

fn console() -> Vec<u64> {
    console_payload("lv2_timer_time_tty")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

#[test]
fn the_timebase_frequency_is_the_word_the_console_returns() {
    let w = console();
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let Lv2Dispatch::Immediate { code, effects } =
        host.dispatch(Lv2Request::TimeGetTimebaseFrequency, UnitId::new(0), &rt)
    else {
        panic!("expected Immediate");
    };
    assert!(effects.is_empty(), "no write");
    assert_eq!(code, w[20], "sc 147");
}

#[test]
fn a_one_second_sleep_on_the_console_measures_the_same_frequency() {
    let w = console();
    assert_eq!(w[4], 0, "sleep(1) returned CELL_OK");
    let measured = w[5];
    let hz = cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
    // A sleep runs at least its length and wakes on the next tick, so the
    // measurement may only overshoot, and by under 0.1%.
    assert!(
        measured >= hz && measured - hz < hz / 1000,
        "{measured} ticks against {hz} Hz"
    );
}
