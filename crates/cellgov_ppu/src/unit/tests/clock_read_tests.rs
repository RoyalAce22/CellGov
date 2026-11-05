//! The per-step guest-clock flag: what sets it, which yield publishes
//! it, and what a fault leaves behind.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

/// `mftb rT`: XFX with TBR 268, encoded low five bits first
/// (`ra` = 12, `rb` = 8).
fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
}

/// `mftbu rT`: the same form with TBR 269.
fn mftbu(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (13 << 16) | (8 << 11) | (371 << 1)
}

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

/// `sc` with LEV 0.
const SC: u32 = (17 << 26) | 2;

/// `lwz r6, 0(r5)`.
const LWZ_R6_R5: u32 = (32 << 26) | (6 << 21) | (5 << 16);

const UNMAPPED_EA: u64 = 0xFFFF_0000;

fn memory_of(words: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::new(256);
    for (index, raw) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(index as u64 * 4), 4).unwrap();
        mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
    }
    mem
}

fn clock_reads(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::ClockRead { .. }))
        .count()
}

#[test]
fn a_batch_that_reads_the_time_base_publishes_one_clock_read() {
    let mem = memory_of(&[mftb(3), addi_inc(4)]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(2), &ctx, &mut effects);

    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    let read = Effect::ClockRead {
        source: UnitId::new(0),
    };
    assert!(effects.contains(&read), "effects: {effects:?}");
    assert_eq!(clock_reads(&effects), 1);
}

#[test]
fn mftbu_publishes_a_clock_read_too() {
    let mem = memory_of(&[mftbu(3), addi_inc(4)]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(2), &ctx, &mut effects);

    assert_eq!(clock_reads(&effects), 1);
}

#[test]
fn two_reads_in_one_batch_publish_one_clock_read() {
    let mem = memory_of(&[mftb(3), mftb(4)]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(2), &ctx, &mut effects);

    assert_eq!(
        clock_reads(&effects),
        1,
        "the flag names the step, not the instruction",
    );
}

#[test]
fn a_step_that_reads_no_time_base_publishes_none() {
    let mem = memory_of(&[addi_inc(3), addi_inc(4)]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem)
        .with_current_tick(GuestTicks::new(cellgov_time::TICKS_PER_SECOND));
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(2), &ctx, &mut effects);

    assert_eq!(unit.state().tb, 0, "the step left the time base alone");
    assert_eq!(unit.time_base_read(), None);
    assert_eq!(clock_reads(&effects), 0);
}

#[test]
fn the_step_after_a_read_publishes_none() {
    let mem = memory_of(&[mftb(3), addi_inc(4)]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);

    let mut first = Vec::new();
    unit.run_until_yield(Budget::new(1), &ctx, &mut first);
    assert_eq!(clock_reads(&first), 1);

    let mut second = Vec::new();
    unit.run_until_yield(Budget::new(1), &ctx, &mut second);
    assert_eq!(
        clock_reads(&second),
        0,
        "the read belongs to the step that took it",
    );
}

#[test]
fn a_read_before_a_syscall_yield_still_publishes() {
    let mem = memory_of(&[mftb(3), SC]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(4), &ctx, &mut effects);

    assert_eq!(result.yield_reason, YieldReason::Syscall);
    assert_eq!(clock_reads(&effects), 1);
}

#[test]
fn the_flag_is_excluded_from_the_state_hash() {
    let base = crate::state::PpuState::new();
    let baseline = base.state_hash();

    let mut with_read = base.clone();
    with_read.clock_read = true;
    assert_eq!(with_read.state_hash(), baseline);
}

#[test]
fn a_read_before_a_fault_is_published_by_the_batch_that_retired_it() {
    let mem = memory_of(&[mftb(3), LWZ_R6_R5]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, UNMAPPED_EA);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let prefix = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(prefix.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(clock_reads(&effects), 1, "the retired mftb is published");

    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert!(effects.is_empty(), "leaked effects: {effects:?}");
    assert!(!unit.state().clock_read);
}
