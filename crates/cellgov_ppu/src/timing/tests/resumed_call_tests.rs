use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::lv2::syscall as sc;

/// Retire `insn` at `pc` as straight-line code.
fn retire(timing: &mut PpeTiming, state: &PpuState, pc: u64, insn: &I) {
    let (ops, n) = classify(insn, state);
    timing.retire_instruction(pc, &ops[..n], pc + 4 * n.max(1) as u64);
}

/// The cycle after one call of `number` and one `addi` behind it, on a
/// thread with no sibling. With `resumed`, the thread left the PPE before
/// the call returned; with `early`, the runtime said so before the call.
pub(super) fn cycle_after_call(number: u64, resumed: bool, early: bool) -> u64 {
    cycle_after_returned_call(number, None, resumed.then_some(false), early)
}

/// [`cycle_after_call`] with the call's committed return `code` noted
/// ahead of the resume, in the order the unit applies its inputs. With
/// `resumed` set, the thread left the PPE; `Some(true)` means it left only
/// for a higher-priority thread.
pub(super) fn cycle_after_returned_call(
    number: u64,
    code: Option<u64>,
    resumed: Option<bool>,
    early: bool,
) -> u64 {
    let mut state = PpuState::new();
    state.set_gpr(11, number);
    let mut timing = PpeTiming::new();
    if early {
        timing.note_syscall_resumed(false);
    }
    retire(&mut timing, &state, 0x1000, &I::Sc { lev: 0 });
    if let Some(code) = code {
        timing.note_syscall_return(code);
    }
    if let Some(preempted) = resumed {
        timing.note_syscall_resumed(preempted);
    }
    timing.charge_syscall();
    retire(
        &mut timing,
        &state,
        0x1004,
        &I::Addi {
            rt: 17,
            ra: 17,
            imm: 1,
        },
    );
    timing.now()
}

#[test]
fn a_yield_that_lost_its_hardware_thread_takes_the_switched_trip() {
    let extra = k::PPU_SYSCALL_SWITCHED_YIELD_ROUND_TRIP_CYCLES
        - k::ppu_syscall_round_trip_cycles(sc::PPU_THREAD_YIELD);
    assert_eq!(
        cycle_after_call(sc::PPU_THREAD_YIELD, true, false)
            - cycle_after_call(sc::PPU_THREAD_YIELD, false, false),
        extra,
    );
}

#[test]
fn a_woken_wait_takes_the_woken_trip() {
    let extra = k::ppu_syscall_resumed_round_trip_cycles(sc::SEMAPHORE_WAIT, None, false)
        - k::ppu_syscall_round_trip_cycles(sc::SEMAPHORE_WAIT);
    assert_eq!(
        cycle_after_call(sc::SEMAPHORE_WAIT, true, false)
            - cycle_after_call(sc::SEMAPHORE_WAIT, false, false),
        extra,
    );
}

#[test]
fn each_woken_wait_takes_its_own_trip() {
    // The cycles of the call's thread beside its round trip.
    let base = cycle_after_call(sc::SEMAPHORE_WAIT, false, false)
        - k::ppu_syscall_round_trip_cycles(sc::SEMAPHORE_WAIT);
    for &(number, trip) in &k::PPU_SYSCALL_WOKEN_ROUND_TRIPS {
        assert_eq!(
            cycle_after_call(number, true, false),
            base + trip,
            "syscall {number}"
        );
    }
    assert_eq!(
        cycle_after_call(sc::MUTEX_LOCK, true, false),
        base + k::PPU_SYSCALL_WOKEN_ROUND_TRIP_CYCLES,
        "a woken wait no capture times takes the default trip",
    );
}

#[test]
fn a_resume_before_the_call_leaves_the_call_s_own_trip() {
    let early = cycle_after_call(sc::SEMAPHORE_WAIT, false, true);
    assert_eq!(early, cycle_after_call(sc::SEMAPHORE_WAIT, false, false));
    assert_eq!(
        cycle_after_call(sc::SEMAPHORE_WAIT, true, false) - early,
        k::ppu_syscall_resumed_round_trip_cycles(sc::SEMAPHORE_WAIT, None, false)
            - k::ppu_syscall_round_trip_cycles(sc::SEMAPHORE_WAIT),
        "a resume after the call charges the woken trip over the early one",
    );
}

#[test]
fn a_woken_wait_with_its_committed_return_takes_the_woken_trip() {
    let woken = cycle_after_returned_call(sc::SEMAPHORE_WAIT, Some(0), Some(false), false);
    assert_eq!(
        woken,
        cycle_after_call(sc::SEMAPHORE_WAIT, true, false),
        "the committed return does not displace the woken trip",
    );
    assert_eq!(
        woken - cycle_after_returned_call(sc::SEMAPHORE_WAIT, Some(0), None, false),
        k::ppu_syscall_resumed_round_trip_cycles(sc::SEMAPHORE_WAIT, Some(0), false)
            - k::ppu_syscall_outcome_round_trip_cycles(sc::SEMAPHORE_WAIT, 0),
    );
}
