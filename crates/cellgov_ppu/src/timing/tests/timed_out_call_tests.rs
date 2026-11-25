use super::resumed_call_tests::{cycle_after_call, cycle_after_returned_call};
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::lv2::errno::CELL_ETIMEDOUT;
use cellgov_ps3_abi::lv2::syscall as sc;

/// The cycle after a woken call of `number` with no trip of its own.
fn base(number: u64) -> u64 {
    cycle_after_call(number, false, false) - k::ppu_syscall_round_trip_cycles(number)
}

#[test]
fn a_timed_out_wait_takes_its_timed_out_trip() {
    let timed_out = Some(u64::from(CELL_ETIMEDOUT));
    for &(number, trip) in &k::PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS {
        assert_eq!(
            cycle_after_returned_call(number, timed_out, Some(false), false),
            base(number) + trip,
            "syscall {number}",
        );
    }
}

#[test]
fn a_timed_out_wait_no_capture_times_keeps_the_woken_trip() {
    let number = sc::COND_WAIT;
    let Some(&(_, woken)) = k::PPU_SYSCALL_WOKEN_ROUND_TRIPS
        .iter()
        .find(|&&(n, _)| n == number)
    else {
        panic!("a woken condition wait has its own trip");
    };
    assert_eq!(
        cycle_after_returned_call(number, Some(u64::from(CELL_ETIMEDOUT)), Some(false), false),
        base(number) + woken,
    );
}

#[test]
fn a_preempted_wait_that_times_out_takes_its_own_trip_and_the_resume_wait() {
    let code = u64::from(CELL_ETIMEDOUT);
    for &(number, timed_out) in &k::PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS {
        let own = k::ppu_syscall_outcome_round_trip_cycles(number, code);
        assert_ne!(own, timed_out, "syscall {number}: the two trips differ");
        assert_eq!(
            cycle_after_returned_call(number, Some(code), Some(true), false),
            base(number) + own + k::PPU_PREEMPTED_RESUME_CYCLES,
            "syscall {number}",
        );
    }
}
