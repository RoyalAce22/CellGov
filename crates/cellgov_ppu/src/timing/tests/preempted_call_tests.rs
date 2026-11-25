use super::resumed_call_tests::{cycle_after_call, cycle_after_returned_call};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, ResumeKind, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::lv2::errno::{CELL_ECANCELED, CELL_ESRCH};
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_time::Budget;

/// `li r11, 147; sc; li r4, 1`: one call of
/// `sys_time_get_timebase_frequency` and one instruction behind it.
const CALL_PROGRAM: [u32; 3] = [0x3960_0093, 0x4400_0002, 0x3880_0001];

/// The cycles of the batch after the `sc` of [`CALL_PROGRAM`], when the
/// runtime commits its return 0 with the resume mark `resumed`.
fn batch_after_call(resumed: Option<ResumeKind>) -> u64 {
    let mut mem = GuestMemory::new(0x100);
    for (i, w) in CALL_PROGRAM.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &w.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let first = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(first.yield_reason, YieldReason::Syscall);
    let ctx = ExecutionContext::with_syscall_return(&mem, &[], 0).with_resumed(resumed);
    let next = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(unit.state().gpr[4], 1, "the batch ran the li behind the sc");
    next.consumed_cost.raw()
}

#[test]
fn a_unit_charges_a_preempted_call_the_resume_wait_and_a_woken_call_the_woken_trip() {
    let number = sc::TIME_GET_TIMEBASE_FREQUENCY;
    let own = batch_after_call(None);
    assert_eq!(
        batch_after_call(Some(ResumeKind::Preempted)) - own,
        k::PPU_PREEMPTED_RESUME_CYCLES,
    );
    assert_eq!(
        batch_after_call(Some(ResumeKind::Woken)) - own,
        k::ppu_syscall_resumed_round_trip_cycles(number, Some(0), false)
            - k::ppu_syscall_outcome_round_trip_cycles(number, 0),
    );
}

#[test]
fn a_preempted_call_takes_its_own_trip_and_the_resume_wait() {
    for number in [sc::TIME_GET_TIMEBASE_FREQUENCY, sc::SEMAPHORE_WAIT] {
        assert_eq!(
            cycle_after_returned_call(number, Some(0), Some(true), false),
            cycle_after_returned_call(number, Some(0), None, false)
                + k::PPU_PREEMPTED_RESUME_CYCLES,
            "syscall {number}",
        );
    }
}

#[test]
fn a_thread_preempted_between_calls_waits_the_resume_wait() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let addi = I::Addi {
        rt: 17,
        ra: 17,
        imm: 1,
    };
    // An independent addi behind it, so only the wait holds its issue.
    let other = I::Addi {
        rt: 18,
        ra: 18,
        imm: 1,
    };
    let (ops, n) = classify(&addi, &state);
    timing.retire_instruction(0x1000, &ops[..n], 0x1004);
    timing.note_syscall_resumed(true);
    timing.charge_syscall();
    let (ops, n) = classify(&other, &state);
    let next = timing.retire_instruction(0x1004, &ops[..n], 0x1008)[0];
    let mut woken = PpeTiming::new();
    let (ops, n) = classify(&addi, &state);
    let first = woken.retire_instruction(0x1000, &ops[..n], 0x1004)[0];
    woken.note_syscall_resumed(false);
    woken.charge_syscall();
    let (ops, n) = classify(&other, &state);
    let woken_next = woken.retire_instruction(0x1004, &ops[..n], 0x1008)[0];
    assert!(
        woken_next < first + 4,
        "a woken thread with no call waits for nothing"
    );
    assert_eq!(
        next,
        woken_next + k::PPU_PREEMPTED_RESUME_CYCLES,
        "the addi after the resume issues the resume wait after a woken thread's",
    );
}

#[test]
fn the_batch_after_a_preempted_resume_waits_for_nothing() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let addi = I::Addi {
        rt: 17,
        ra: 17,
        imm: 1,
    };
    let (ops, n) = classify(&addi, &state);
    timing.retire_instruction(0x1000, &ops[..n], 0x1004);
    timing.note_syscall_resumed(true);
    timing.charge_syscall();
    let (ops, n) = classify(&addi, &state);
    let resumed = timing.retire_instruction(0x1004, &ops[..n], 0x1008)[0];
    // The next batch starts with no resume of its own.
    timing.charge_syscall();
    let (ops, n) = classify(&addi, &state);
    let next = timing.retire_instruction(0x1008, &ops[..n], 0x100c)[0];
    assert!(
        next < resumed + 4,
        "the addi of the next batch issues at {next}, the one after the resume at {resumed}",
    );
}

#[test]
fn a_preempted_yield_takes_the_switched_trip() {
    assert_eq!(
        cycle_after_returned_call(sc::PPU_THREAD_YIELD, Some(0), Some(true), false),
        cycle_after_call(sc::PPU_THREAD_YIELD, true, false),
    );
}

#[test]
fn a_cancelled_wait_takes_its_cancelled_trip() {
    let base = cycle_after_call(sc::EVENT_FLAG_WAIT, false, false)
        - k::ppu_syscall_round_trip_cycles(sc::EVENT_FLAG_WAIT);
    let cancelled = Some(u64::from(CELL_ECANCELED));
    for &(number, trip) in &k::PPU_SYSCALL_CANCELLED_ROUND_TRIPS {
        assert_eq!(
            cycle_after_returned_call(number, cancelled, Some(false), false),
            base + trip,
            "syscall {number}",
        );
    }
    // A wait no capture cancels, and a cancelled wait's other errors,
    // keep the woken trip.
    for (number, code) in [
        (sc::SEMAPHORE_WAIT, CELL_ECANCELED),
        (sc::EVENT_FLAG_WAIT, CELL_ESRCH),
    ] {
        let base =
            cycle_after_call(number, false, false) - k::ppu_syscall_round_trip_cycles(number);
        let woken = k::PPU_SYSCALL_WOKEN_ROUND_TRIPS
            .iter()
            .find(|&&(n, _)| n == number)
            .map_or(k::PPU_SYSCALL_WOKEN_ROUND_TRIP_CYCLES, |&(_, trip)| trip);
        assert_eq!(
            cycle_after_returned_call(number, Some(u64::from(code)), Some(false), false),
            base + woken,
            "syscall {number} returning {}",
            code.symbol,
        );
    }
}
