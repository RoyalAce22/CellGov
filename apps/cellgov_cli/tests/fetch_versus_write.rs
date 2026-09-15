//! A unit executing from a region another unit writes.
//!
//! Instruction fetch reads committed memory, and a committed write to
//! the text region drives `invalidate_code` over every unit's shadow,
//! so the next fetch takes the new bytes. The two orders are
//! observably different and the search has to replay the pair rather
//! than prune it.
//!
//! Only the PPU fetches, so this is the one place the pair can be
//! built: the fake ISA interprets a program vector and reads no text.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_explore::{Execution, StepFootprint};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

/// Where the PPU executes from.
const TEXT: u64 = 0x100;

/// `ori r0, r0, 0` -- a no-op that retires and advances the pc.
const NOP: u32 = 0x6000_0000;

fn place(mem: &mut GuestMemory, addr: u64, word: u32) {
    let range = ByteRange::new(GuestAddr::new(addr), 4).unwrap();
    mem.apply_commit(range, &word.to_be_bytes()).unwrap();
}

/// The footprint of one PPU block that runs `budget` instructions from
/// `TEXT`.
fn ppu_block_footprint(budget: u64) -> StepFootprint {
    let mut mem = GuestMemory::new(0x1000);
    for offset in 0..16 {
        place(&mut mem, TEXT + offset * 4, NOP);
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = TEXT;
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    StepFootprint::from_effects(&effects)
}

#[test]
fn a_block_records_the_text_it_fetched() {
    let footprint = ppu_block_footprint(4);
    assert_eq!(
        footprint.shared_reads.len(),
        1,
        "four instructions in one run coalesce to one read",
    );
    let read = footprint.shared_reads[0];
    assert_eq!(read.start().raw(), TEXT);
    assert_eq!(read.length(), 16, "four instructions of four bytes");
}

#[test]
fn a_write_to_the_text_a_block_fetched_conflicts_with_it() {
    let fetching = ppu_block_footprint(4);
    let writing = StepFootprint::from_effects(&[Effect::shared_write(
        ByteRange::new(GuestAddr::new(TEXT + 4), 4).unwrap(),
        cellgov_effects::WritePayload::new(vec![0; 4]),
        UnitId::new(1),
        cellgov_time::GuestTicks::ZERO,
    )]);
    assert!(fetching.conflicts(&writing));
    assert!(writing.conflicts(&fetching));
}

#[test]
fn a_write_clear_of_the_text_a_block_fetched_prunes_against_it() {
    let fetching = ppu_block_footprint(4);
    let writing = StepFootprint::from_effects(&[Effect::shared_write(
        ByteRange::new(GuestAddr::new(TEXT + 0x200), 4).unwrap(),
        cellgov_effects::WritePayload::new(vec![0; 4]),
        UnitId::new(1),
        cellgov_time::GuestTicks::ZERO,
    )]);
    assert!(!fetching.conflicts(&writing));
}

/// The search holds the two units apart rather than pruning them.
#[test]
fn the_relation_holds_a_fetching_unit_and_a_writer_apart() {
    let fetching = ppu_block_footprint(4);
    let writing = StepFootprint::from_effects(&[Effect::shared_write(
        ByteRange::new(GuestAddr::new(TEXT), 4).unwrap(),
        cellgov_effects::WritePayload::new(vec![0; 4]),
        UnitId::new(1),
        cellgov_time::GuestTicks::ZERO,
    )]);
    let mut execution = Execution::new();
    execution.push(UnitId::new(0), fetching);
    execution.push(UnitId::new(1), writing);
    assert!(
        !execution.units_independent(UnitId::new(0), UnitId::new(1)),
        "a fetch of the text and a write to it are not independent",
    );
}

/// A block that branches records each run it touched, not the span
/// between them.
#[test]
fn a_branching_block_records_each_run_it_fetched() {
    let mut mem = GuestMemory::new(0x1000);
    // Two nops, then a branch forward over a gap to two more nops.
    place(&mut mem, TEXT, NOP);
    place(&mut mem, TEXT + 4, NOP);
    // `b +0x80` from TEXT + 8 lands at TEXT + 0x88.
    place(&mut mem, TEXT + 8, 0x4800_0080);
    place(&mut mem, TEXT + 0x88, NOP);
    place(&mut mem, TEXT + 0x8C, NOP);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = TEXT;
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(5), &ctx, &mut effects);
    let footprint = StepFootprint::from_effects(&effects);
    assert_eq!(
        footprint.shared_reads.len(),
        2,
        "the run before the branch and the run after it",
    );
    assert_eq!(footprint.shared_reads[0].start().raw(), TEXT);
    assert_eq!(
        footprint.shared_reads[0].length(),
        12,
        "two nops and the branch",
    );
    assert_eq!(footprint.shared_reads[1].start().raw(), TEXT + 0x88);
    assert_eq!(
        footprint.shared_reads[1].length(),
        8,
        "the two nops the branch landed on",
    );
}

/// A block the fault rolls back retired nothing, so the text it
/// fetched belongs to no effect list -- neither its own, which the
/// rollback clears, nor the next block's.
#[test]
fn a_faulted_block_leaves_no_run_for_the_next_one() {
    // A second stretch of text, far enough from TEXT that a leaked
    // run cannot be mistaken for the next block's own.
    const OTHER: u64 = TEXT + 0x400;

    let mut mem = GuestMemory::new(0x1000);
    for offset in 0..16 {
        place(&mut mem, TEXT + offset * 4, NOP);
        place(&mut mem, OTHER + offset * 4, NOP);
    }

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = TEXT;
    // The break fires at the third instruction, after two fetches the
    // rollback discards.
    unit.set_break_pc(TEXT + 8, 0);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let faulted = unit.run_until_yield(Budget::new(4), &ctx, &mut effects);
    assert_eq!(faulted.yield_reason, cellgov_exec::YieldReason::Fault);
    assert!(effects.is_empty(), "the break discards its block");

    unit.state_mut().pc = OTHER;
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(2), &ctx, &mut effects);
    let footprint = StepFootprint::from_effects(&effects);
    assert_eq!(
        footprint.shared_reads.len(),
        1,
        "the next block publishes its own run alone",
    );
    assert_eq!(footprint.shared_reads[0].start().raw(), OTHER);
    assert_eq!(footprint.shared_reads[0].length(), 8);
}

/// Past the run cap the block keeps one span, and the span still
/// covers every address it fetched.
#[test]
fn a_block_past_the_run_cap_covers_every_address_it_fetched() {
    // Branches taken, several more than the cap the block tracks
    // apart, so the collapse runs and then reopens runs after it.
    const BRANCHES: u64 = 12;

    let mut mem = GuestMemory::new(0x1000);
    // A chain of `b +8`: each branch is its own run, eight bytes on
    // from the last, so the gaps between the runs are never fetched.
    for i in 0..BRANCHES {
        place(&mut mem, TEXT + i * 8, 0x4800_0008);
    }

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = TEXT;
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(BRANCHES), &ctx, &mut effects);
    let footprint = StepFootprint::from_effects(&effects);

    let last = TEXT + (BRANCHES - 1) * 8;
    for i in 0..BRANCHES {
        let fetched = ByteRange::new(GuestAddr::new(TEXT + i * 8), 4).unwrap();
        assert!(
            footprint
                .shared_reads
                .iter()
                .any(|read| read.overlaps(fetched)),
            "no published read covers the fetch at {:#x}",
            TEXT + i * 8,
        );
    }
    assert!(
        footprint
            .shared_reads
            .iter()
            .all(|read| read.end().raw() <= last + 4),
        "no published read reaches past the last address fetched",
    );
}

/// A runtime still steps the pair: the commit pipeline takes the
/// effect and stages nothing for it.
#[test]
fn the_read_intent_reaches_a_commit() {
    let mut mem = GuestMemory::new(0x1000);
    for offset in 0..8 {
        place(&mut mem, TEXT + offset * 4, NOP);
    }
    let mut rt = Runtime::new(mem, Budget::new(4), 8);
    rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        unit.state_mut().pc = TEXT;
        unit
    });
    let step = rt.step().expect("the unit is runnable");
    assert!(
        step.effects
            .iter()
            .any(|e| matches!(e, Effect::SharedReadIntent { .. })),
        "the block published what it fetched",
    );
    rt.commit_step(&step.result, &step.effects)
        .expect("a read intent stages nothing and cannot be refused");
}
