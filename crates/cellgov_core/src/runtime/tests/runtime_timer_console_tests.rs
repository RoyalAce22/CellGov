//! The timer sleeps held to the `lv2_timer_time_tty` capture from the
//! reference console: each sleep returns the console's r3, a zero
//! interval returns without a park, and a nonzero one parks the caller
//! for at least the interval in guest time.

use super::*;

use cellgov_ps3_abi::lv2::syscall::{TIMER_SLEEP, TIMER_USLEEP};

/// Each sleep the probe ran, its argument, the interval it asks for in
/// microseconds, and the payload slot of its return code; the slot after
/// it holds the timebase ticks the console spent.
const SLEEPS: [(u64, u64, u64, usize); 4] = [
    (TIMER_USLEEP, 1000, 1000, 0),
    (TIMER_USLEEP, 0, 0, 2),
    (TIMER_SLEEP, 1, 1_000_000, 4),
    (TIMER_SLEEP, 0, 0, 6),
];

fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_timer_time_tty/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

#[test]
fn each_sleep_returns_and_parks_as_on_the_console() {
    let console = console_words();
    let tb_hz = cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
    for (num, arg, usec, slot) in SLEEPS {
        let console_rc = console[slot];
        let console_usec = console[slot + 1] * 1_000_000 / tb_hz;
        assert!(
            console_usec >= usec,
            "console slept {console_usec} us for {usec}"
        );

        let mut rt = build(4096, 16, 100);
        let unit_id = rt
            .registry_mut()
            .register_with(|id| SleepingUnit::new(id, num, arg));
        let s = rt.step().unwrap();
        rt.commit_unchecked(&s.result, &s.effects).unwrap();
        let parked = rt.registry().effective_status(unit_id) == Some(UnitStatus::Blocked);
        let park_time = rt.time();
        assert_eq!(parked, usec != 0, "sc {num}({arg}): parked");

        let s2 = rt.step().unwrap();
        rt.commit_unchecked(&s2.result, &s2.effects).unwrap();
        let slept_usec = (rt.time().raw() - park_time.raw()) / cellgov_time::TICKS_PER_USEC;
        assert!(
            slept_usec >= usec,
            "sc {num}({arg}): CellGov slept {slept_usec} us"
        );
        let unit = rt.registry().get(unit_id).unwrap();
        let sleeper = unit.as_any().downcast_ref::<SleepingUnit>().unwrap();
        assert_eq!(
            sleeper.observed_r3.get(),
            Some(console_rc),
            "sc {num}({arg}): r3"
        );
    }
}
