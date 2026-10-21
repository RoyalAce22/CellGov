//! Which SPU steps publish a `ClockRead` effect: the steps whose result
//! depends on the tick they run at, through the decrementer.

use crate::SpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    event::{S1, TM},
    SPU_RD_DEC, SPU_RD_EVENT_STAT, SPU_WR_DEC, SPU_WR_EVENT_ACK, SPU_WR_EVENT_MASK,
};
use cellgov_time::{Budget, GuestTicks};

const UNIT: UnitId = UnitId::new(3);
const NOP: u32 = 0x4020_0000;
const STOP: u32 = 0;

const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

const fn rchcnt(channel: u8, rt: u32) -> u32 {
    (0x00F << 21) | ((channel as u32) << 7) | rt
}

/// A unit that runs `words` from address 0, with r10 holding Tm.
fn unit_with(words: &[u32]) -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UNIT);
    let state = unit.state_mut();
    for (i, word) in words.iter().enumerate() {
        state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    state.set_reg_word_splat(10, TM);
    unit
}

/// Counts the `ClockRead` effects that one step of `budget` instructions
/// publishes at tick `at`.
fn clock_reads_of_step(unit: &mut SpuExecutionUnit, budget: u64, at: u64) -> usize {
    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem).with_current_tick(GuestTicks::new(at));
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    effects
        .iter()
        .filter(|effect| **effect == Effect::ClockRead { source: UNIT })
        .count()
}

/// Runs the one-nop step that anchors the count, so the next step shows
/// what its own instructions read.
fn anchored(words: &[u32]) -> SpuExecutionUnit {
    let mut program = vec![NOP];
    program.extend_from_slice(words);
    let mut unit = unit_with(&program);
    assert_eq!(
        clock_reads_of_step(&mut unit, 1, 1_000),
        1,
        "the first look at the count anchors it at the step's tick",
    );
    unit
}

#[test]
fn the_step_that_anchors_the_count_reads_the_clock_and_the_next_does_not() {
    let mut unit = anchored(&[NOP, NOP, STOP]);
    assert_eq!(clock_reads_of_step(&mut unit, 3, 2_000), 0);
}

#[test]
fn a_read_of_a_running_count_publishes_one_clock_read_per_step() {
    let mut unit = anchored(&[rdch(SPU_RD_DEC, 2), rdch(SPU_RD_DEC, 3), STOP]);
    assert_eq!(
        clock_reads_of_step(&mut unit, 3, 2_000),
        1,
        "the effect names the step, not the read",
    );
}

#[test]
fn a_load_of_the_count_reads_the_clock() {
    let mut unit = anchored(&[wrch(SPU_WR_DEC, 11), STOP]);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 1);
}

#[test]
fn a_read_of_a_stopped_count_does_not_read_the_clock() {
    let mut unit = unit_with(&[rdch(SPU_RD_DEC, 2), STOP]);
    // A stop at time base 0 anchors the count and freezes it.
    unit.state_mut().channels.decrementer.stop(0);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 0);
}

#[test]
fn a_look_at_the_event_status_with_tm_enabled_reads_the_clock() {
    let mut unit = anchored(&[rchcnt(SPU_RD_EVENT_STAT, 2), STOP]);
    unit.state_mut().channels.event_mask = TM;
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 1);
}

#[test]
fn a_look_at_the_event_status_with_tm_masked_does_not() {
    let mut unit = anchored(&[rchcnt(SPU_RD_EVENT_STAT, 2), STOP]);
    unit.state_mut().channels.event_mask = S1;
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 0);
}

#[test]
fn a_look_at_the_event_status_with_tm_already_pending_does_not() {
    let mut unit = anchored(&[rdch(SPU_RD_EVENT_STAT, 2), STOP]);
    let channels = &mut unit.state_mut().channels;
    channels.event_mask = TM;
    channels.set_event_state(TM, TM);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 0);
}

#[test]
fn a_mask_write_and_an_acknowledgment_read_the_clock_while_tm_can_rise() {
    for word in [wrch(SPU_WR_EVENT_MASK, 10), wrch(SPU_WR_EVENT_ACK, 10)] {
        let mut unit = anchored(&[word, STOP]);
        assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 1, "{word:#x}");
    }
}

#[test]
fn the_step_whose_entry_finds_the_zero_crossing_reads_the_clock() {
    let mut unit = anchored(&[wrch(SPU_WR_DEC, 11), rchcnt(SPU_RD_EVENT_STAT, 2), STOP]);
    unit.state_mut().set_reg_word_splat(11, 1);
    assert_eq!(clock_reads_of_step(&mut unit, 1, 2_000), 1, "the load");
    unit.state_mut().channels.event_mask = TM;
    // The count of 1 passed zero long before this tick, so the entry
    // raises Tm and the count look sees it already pending.
    assert_eq!(clock_reads_of_step(&mut unit, 2, 1_000_000), 1);
    assert_ne!(unit.state_mut().channels.pending_events & TM, 0);
}

#[test]
fn an_acknowledgment_that_stops_the_count_reads_the_clock_with_tm_pending() {
    let mut unit = anchored(&[wrch(SPU_WR_EVENT_ACK, 10), STOP]);
    // Tm is pending and masked, so the ack with r10 = Tm stops the count.
    unit.state_mut().channels.set_event_state(TM, 0);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 1);
    assert!(!unit.state_mut().channels.decrementer.running());
}

#[test]
fn an_interrupt_check_with_tm_enabled_reads_the_clock() {
    let mut unit = anchored(&[NOP, STOP]);
    let state = unit.state_mut();
    state.channels.event_mask = TM;
    state.set_interrupts_enabled(true);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 1);
}

#[test]
fn an_interrupt_check_with_tm_masked_does_not() {
    let mut unit = anchored(&[NOP, STOP]);
    let state = unit.state_mut();
    state.channels.event_mask = S1;
    state.set_interrupts_enabled(true);
    assert_eq!(clock_reads_of_step(&mut unit, 2, 2_000), 0);
}
