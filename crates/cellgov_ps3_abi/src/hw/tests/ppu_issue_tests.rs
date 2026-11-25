use super::*;
use crate::lv2::errno::{CELL_EBUSY, CELL_EDEADLK, CELL_EPERM, CELL_ESRCH};

#[test]
fn an_error_past_the_id_lookup_no_capture_times_keeps_the_call_s_success_trip() {
    let timed = |number: u64, code: u32| {
        PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS
            .iter()
            .chain(&PPU_SYSCALL_REFUSAL_ROUND_TRIPS)
            .any(|&(n, c, _)| n == number && c == code)
    };
    let mut checked = 0;
    for &(number, _) in &PPU_SYSCALL_ERROR_ROUND_TRIPS {
        for code in [CELL_EBUSY, CELL_EDEADLK, CELL_EPERM] {
            if timed(number, code.code) {
                continue;
            }
            checked += 1;
            assert_eq!(
                ppu_syscall_outcome_round_trip_cycles(number, u64::from(code)),
                ppu_syscall_round_trip_cycles(number),
                "syscall {number} returning {}",
                code.symbol,
            );
        }
    }
    // 13 calls by three codes, less the seven pairs a capture times.
    assert_eq!(checked, 32);
}

#[test]
fn an_untimed_call_takes_the_lower_median_of_the_timed_success_trips() {
    let mut trips: Vec<u64> = PPU_SYSCALL_ROUND_TRIPS.iter().map(|&(_, c)| c).collect();
    trips.sort_unstable();
    assert_eq!(
        PPU_SYSCALL_UNTIMED_ROUND_TRIP_CYCLES,
        trips[(trips.len() - 1) / 2]
    );
    assert_eq!(
        ppu_syscall_round_trip_cycles(crate::lv2::syscall::PPU_THREAD_CREATE),
        PPU_SYSCALL_UNTIMED_ROUND_TRIP_CYCLES,
        "thread create is untimed",
    );
}

#[test]
fn a_sign_extended_esrch_takes_the_error_trip() {
    for &(number, cycles) in &PPU_SYSCALL_ERROR_ROUND_TRIPS {
        assert_eq!(
            ppu_syscall_outcome_round_trip_cycles(number, u64::from(CELL_ESRCH)),
            cycles,
            "syscall {number}",
        );
    }
}
