//! Where a PPE thread's batch ends while a sibling thread is runnable: at
//! a grid line of the thread's own clock.

use super::batch::SIBLING_LEAD_CYCLES;
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;

/// The instructions a batch may retire.
const BUDGET: u64 = 256;

/// One dependent `addi` and the sibling's hold past it, at most: the
/// cycles the instruction that reaches the line can run past it.
const LAST_INSN_CYCLES: u64 = 8;

/// A dependent `addi` at each word the batch can reach.
fn chain() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    for index in 0..BUDGET {
        let range = ByteRange::new(GuestAddr::new(index * 4), 4).expect("range");
        mem.apply_commit(range, &ADDI_R3.to_be_bytes())
            .expect("mapped");
    }
    mem
}

/// One batch of the chain from tick `start` beside `siblings` runnable
/// PPE siblings: the instructions it retired and the cycles it took.
fn batch(start: u64, siblings: u32) -> (u64, u64) {
    batch_of(start, siblings, BUDGET)
}

/// [`batch`] with a budget of `budget` instructions.
fn batch_of(start: u64, siblings: u32, budget: u64) -> (u64, u64) {
    let mem = chain();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem)
        .with_current_tick(GuestTicks::new(start))
        .with_ppe_siblings(siblings);
    let result = unit.run_until_yield(Budget::new(budget), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    (unit.state().pc / 4, result.consumed_cost.raw())
}

#[test]
fn a_batch_that_starts_on_a_grid_line_ends_at_the_next() {
    let (retired, cycles) = batch(4 * SIBLING_LEAD_CYCLES, 1);
    assert!(
        retired < BUDGET,
        "the batch retired its whole budget in {cycles} cycles"
    );
    assert!(
        (SIBLING_LEAD_CYCLES..SIBLING_LEAD_CYCLES + LAST_INSN_CYCLES).contains(&cycles),
        "the batch took {cycles} cycles"
    );
}

#[test]
fn a_batch_that_starts_between_grid_lines_ends_at_the_next_line() {
    let into = 100;
    let (retired, cycles) = batch(4 * SIBLING_LEAD_CYCLES + into, 1);
    let to_line = SIBLING_LEAD_CYCLES - into;
    assert!(retired < BUDGET);
    assert!(
        (to_line..to_line + LAST_INSN_CYCLES).contains(&cycles),
        "the batch took {cycles} cycles"
    );
}

#[test]
fn a_batch_whose_instruction_reaches_a_grid_line_exactly_ends_on_it() {
    // A short batch from a line ends on its budget before the next line.
    // A batch that starts that many cycles short of a line reaches the
    // line with the same instruction and retires nothing past it.
    let short = 10;
    let (retired, to_line) = batch_of(0, 1, short);
    assert_eq!(retired, short);
    assert!(to_line < SIBLING_LEAD_CYCLES);
    let (retired, cycles) = batch(5 * SIBLING_LEAD_CYCLES - to_line, 1);
    assert_eq!(
        (retired, cycles),
        (short, to_line),
        "the batch ran past the line it reached"
    );
}

#[test]
fn the_line_a_batch_ends_at_moves_with_its_own_clock_and_not_its_sibling_count() {
    // The rule reads the thread's own clock and whether a sibling is
    // runnable. Two siblings end the batch at the same line one does.
    let start = 7 * SIBLING_LEAD_CYCLES + 40;
    let one = batch(start, 1);
    let two = batch(start, 2);
    assert_eq!(one.0, two.0, "the batch retired as many instructions");
    let later = batch(start + SIBLING_LEAD_CYCLES, 1);
    assert_eq!(one.0, later.0);
}

#[test]
fn a_batch_with_no_runnable_sibling_runs_its_whole_budget() {
    let (retired, cycles) = batch(1000, 0);
    assert_eq!(retired, BUDGET);
    assert!(
        cycles > SIBLING_LEAD_CYCLES,
        "the chain is long enough to pass a line: {cycles} cycles"
    );
}
