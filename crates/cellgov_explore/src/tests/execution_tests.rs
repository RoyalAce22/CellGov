//! Happens-before and the race query over hand-computed executions,
//! each fixture asserting the whole race set.

use super::*;
use crate::observer::observe_decisions;
use crate::util::StopReason;
use cellgov_core::Runtime;
use cellgov_effects::payload::WritePayload;
use cellgov_effects::Effect;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::fixtures::write_conflict_scenario;
use cellgov_testkit::world::WritingUnit;
use cellgov_time::{Budget, GuestTicks};

const MEASURED_STEPS_PER_UNIT: u64 = 512;

const MEASURED_DISJOINT_UNITS: usize = 4;

fn unit(raw: u64) -> UnitId {
    UnitId::new(raw)
}

fn range(start: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), len).unwrap()
}

fn writes(start: u64) -> StepFootprint {
    StepFootprint::from_effects(&[Effect::shared_write(
        range(start, 4),
        WritePayload::new(vec![0; 4]),
        unit(0),
        GuestTicks::ZERO,
    )])
}

fn local() -> StepFootprint {
    StepFootprint::default()
}

fn race(first: (usize, u64), second: (usize, u64)) -> Race {
    Race {
        first: EventId {
            index: first.0,
            unit: unit(first.1),
        },
        second: EventId {
            index: second.0,
            unit: unit(second.1),
        },
    }
}

fn races_of(execution: &Execution) -> BTreeSet<Race> {
    execution.races(&execution.happens_before())
}

fn two_writers_of_one_word() -> Execution {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    execution
}

fn two_writers_of_separate_words() -> Execution {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(8));
    execution
}

fn three_writers_of_one_word() -> Execution {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    execution.push(unit(2), writes(0));
    execution
}

#[test]
fn two_units_writing_one_word_race() {
    assert_eq!(
        races_of(&two_writers_of_one_word()),
        BTreeSet::from([race((0, 0), (1, 1))]),
    );
}

#[test]
fn two_units_writing_separate_words_race_over_nothing() {
    assert_eq!(races_of(&two_writers_of_separate_words()), BTreeSet::new());
}

#[test]
fn a_conflict_ordered_through_a_third_event_is_not_a_race() {
    // 0 -> 1 and 1 -> 2 are adjacent; 0 -> 2 reaches through 1, so
    // reversing that pair is not a choice the schedule holds.
    assert_eq!(
        races_of(&three_writers_of_one_word()),
        BTreeSet::from([race((0, 0), (1, 1)), race((1, 1), (2, 2))]),
    );
}

#[test]
fn the_event_between_a_pair_may_belong_to_the_later_event_s_own_unit() {
    // Unit 1 writes the word twice. Its second write reaches unit 0's
    // write through its own first one, so only the first is a race.
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    execution.push(unit(1), writes(0));
    assert_eq!(races_of(&execution), BTreeSet::from([race((0, 0), (1, 1))]),);
}

#[test]
fn only_the_latest_conflicting_event_of_a_unit_races_with_a_later_one() {
    // Unit 0 writes the word twice before unit 1 writes it. The first
    // write reaches unit 1's write through the second one, so the pair
    // the schedule holds apart is the second write against unit 1's.
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    assert_eq!(races_of(&execution), BTreeSet::from([race((1, 0), (2, 1))]));
}

#[test]
fn a_later_event_of_the_earlier_unit_that_conflicts_with_nothing_hides_no_race() {
    // Unit 0 writes one word then another; unit 1 writes the first.
    // The clock of unit 1's event names unit 0's first write, not its
    // second, so the second is no event between the pair.
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(0), writes(8));
    execution.push(unit(1), writes(0));
    assert_eq!(races_of(&execution), BTreeSet::from([race((0, 0), (2, 1))]));
}

#[test]
fn two_events_of_one_unit_on_one_address_race_over_nothing() {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(8));
    assert_eq!(races_of(&execution), BTreeSet::new());
}

#[test]
fn a_local_only_step_orders_nothing_and_hides_no_race_across_itself() {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(0), local());
    execution.push(unit(1), writes(0));
    let hb = execution.happens_before();
    assert!(
        !hb.precedes(
            EventId {
                index: 1,
                unit: unit(0)
            },
            EventId {
                index: 2,
                unit: unit(1)
            },
        ),
        "a step that touches nothing shared orders nothing after it",
    );
    assert_eq!(execution.races(&hb), BTreeSet::from([race((0, 0), (2, 1))]));
}

/// A footprint whose step writes the bytes of a transfer in flight.
fn rides_a_landing(addr: u64) -> StepFootprint {
    let mut fp = writes(addr);
    fp.inflight_dma_ranges.push(range(addr, 4));
    fp
}

#[test]
fn a_step_riding_a_landing_races_with_one_that_touched_nothing() {
    // Every step carries ticks, so a step that touched nothing still
    // decides which side of the landing the riding step falls on.
    for (first, second, name) in [
        (rides_a_landing(0), local(), "rider first"),
        (local(), rides_a_landing(0), "rider second"),
    ] {
        let mut execution = Execution::new();
        execution.push(unit(0), first);
        execution.push(unit(1), second);
        let hb = execution.happens_before();
        assert_eq!(
            execution.races(&hb),
            BTreeSet::from([race((0, 0), (1, 1))]),
            "{name}: the pair is a race the search is owed",
        );
    }
}

#[test]
fn two_steps_that_touched_nothing_are_still_independent() {
    // The conflict scan visits every event, so the footprint test is
    // the whole of what prunes this pair.
    let mut execution = Execution::new();
    execution.push(unit(0), local());
    execution.push(unit(1), local());
    let hb = execution.happens_before();
    assert_eq!(execution.races(&hb), BTreeSet::new());
    assert!(execution.units_independent(unit(0), unit(1)));
}

#[test]
fn a_conflict_orders_its_two_events_and_nothing_orders_a_disjoint_pair() {
    let execution = two_writers_of_one_word();
    let hb = execution.happens_before();
    let first = EventId {
        index: 0,
        unit: unit(0),
    };
    let second = EventId {
        index: 1,
        unit: unit(1),
    };
    assert!(hb.precedes(first, second));
    assert!(!hb.precedes(second, first), "the relation is asymmetric");

    let disjoint = two_writers_of_separate_words();
    let hb = disjoint.happens_before();
    assert!(!hb.precedes(first, second));
}

#[test]
fn the_closure_carries_an_order_through_a_third_unit() {
    // 0 conflicts with 1, 1 conflicts with 2, and 0 touches nothing 2
    // touches -- so only the closure puts 0 before 2.
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    execution.push(unit(1), writes(8));
    execution.push(unit(2), writes(8));
    let hb = execution.happens_before();
    assert!(hb.precedes(
        EventId {
            index: 0,
            unit: unit(0)
        },
        EventId {
            index: 3,
            unit: unit(2)
        },
    ));
}

#[test]
fn a_clock_names_the_latest_event_of_each_unit_it_reaches() {
    let execution = three_writers_of_one_word();
    let hb = execution.happens_before();
    let clock = hb.clock(2).expect("the third event has a clock");
    assert_eq!(clock.get(unit(0)), Some(0));
    assert_eq!(clock.get(unit(1)), Some(1));
    assert_eq!(clock.get(unit(2)), Some(2));
    assert_eq!(clock.len(), 3);

    let first = hb.clock(0).expect("the first event has a clock");
    assert_eq!(first.get(unit(1)), None);
    assert!(!first.is_empty(), "a clock names its own event");
    assert_eq!(
        first.entries().collect::<Vec<_>>(),
        vec![(unit(0), 0)],
        "a clock reached by nothing carries only its own entry",
    );
}

#[test]
#[should_panic(expected = "the relation covers a different execution")]
fn a_relation_built_over_another_execution_is_refused() {
    let short = two_writers_of_one_word();
    let long = three_writers_of_one_word();
    let _ = short.races(&long.happens_before());
}

#[test]
#[should_panic(expected = "the relation covers a different execution at event 0")]
fn a_relation_built_over_another_unit_order_is_refused() {
    // Same event count, the two units swapped. A length check alone
    // lets this answer every race out of the wrong clocks.
    let mut swapped = Execution::new();
    swapped.push(unit(1), writes(0));
    swapped.push(unit(0), writes(0));
    let _ = two_writers_of_one_word().races(&swapped.happens_before());
}

#[test]
#[should_panic(expected = "the relation covers no EventId")]
fn an_event_the_relation_does_not_cover_is_refused_rather_than_unordered() {
    let execution = two_writers_of_one_word();
    let hb = execution.happens_before();
    let _ = hb.precedes(
        EventId {
            index: 0,
            unit: unit(1),
        },
        EventId {
            index: 1,
            unit: unit(1),
        },
    );
}

#[test]
fn a_unit_that_never_ran_is_independent_of_nothing() {
    let execution = two_writers_of_separate_words();
    assert!(execution.units_independent(unit(0), unit(1)));
    assert!(
        !execution.units_independent(unit(0), unit(7)),
        "a unit with no event recorded nothing to prune on",
    );
}

#[test]
fn a_unit_whose_every_step_touched_nothing_shared_is_independent_of_all() {
    let mut execution = Execution::new();
    execution.push(unit(0), local());
    execution.push(unit(0), local());
    execution.push(unit(1), writes(0));
    assert!(execution.units_independent(unit(0), unit(1)));
}

#[test]
fn a_unit_conflicting_on_one_of_its_steps_is_not_independent() {
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(8));
    execution.push(unit(1), writes(0));
    assert!(!execution.units_independent(unit(0), unit(1)));
}

#[test]
fn the_execution_a_log_records_names_every_step_in_order() {
    let mut rt = write_conflict_scenario(3).build_runtime();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let execution = Execution::from_log(&log);

    assert_eq!(execution.len(), log.len());
    assert!(!execution.is_empty());
    assert_eq!(
        execution.units().collect::<Vec<_>>(),
        vec![unit(0), unit(1)]
    );
    for (position, event) in execution.events().iter().enumerate() {
        assert_eq!(event.id.index, position);
        assert_eq!(event.id.unit, log.points()[position].chosen);
    }
    assert_eq!(execution.events_of(unit(0)).count(), 3);
}

/// `MEASURED_DISJOINT_UNITS` writers, each on its own word, so no
/// conflict ever cuts the backward scan short.
fn disjoint_writers(steps_per_unit: u64) -> Runtime {
    let units = MEASURED_DISJOINT_UNITS;
    let mut rt = Runtime::new(
        GuestMemory::new(256),
        Budget::new(1),
        units * steps_per_unit as usize + 1,
    );
    for slot in 0..units {
        let word = range(slot as u64 * 8, 4);
        rt.register_unit_with(move |id| WritingUnit::new(id, steps_per_unit, word));
    }
    rt
}

/// The cost an epoch representation shrinks [FlanaganFreund2009 p:2 s:1] is
/// the clock-vector width and the join count. Neither grows with the
/// run: a clock vector holds one entry per unit, and a run has a
/// handful of units. The conflict scan does grow, and no clock-vector
/// representation touches it.
#[test]
fn every_step_conflicting_costs_one_conflict_test_and_one_join_per_event() {
    let mut rt = write_conflict_scenario(MEASURED_STEPS_PER_UNIT).build_runtime();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let execution = Execution::from_log(&log);
    assert_eq!(execution.len(), 2 * MEASURED_STEPS_PER_UNIT as usize);

    let cost = execution.happens_before().cost();
    assert_eq!(cost.widest_clock, 2);
    assert_eq!(cost.joins, 1023);
    assert_eq!(cost.conflict_tests, 1023);
}

#[test]
fn a_conflict_the_clock_already_carries_costs_no_footprint_test() {
    // Unit 1's second event reaches unit 0's write through its own
    // first event, so the backward scan stops before testing it: one
    // test for the whole execution, not two.
    let mut execution = Execution::new();
    execution.push(unit(0), writes(0));
    execution.push(unit(1), writes(0));
    execution.push(unit(1), writes(0));
    assert_eq!(execution.happens_before().cost().conflict_tests, 1);
}

#[test]
fn a_run_whose_units_never_conflict_scans_every_earlier_event_of_every_other_unit() {
    let mut rt = disjoint_writers(MEASURED_STEPS_PER_UNIT);
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let execution = Execution::from_log(&log);
    assert_eq!(
        execution.len(),
        MEASURED_DISJOINT_UNITS * MEASURED_STEPS_PER_UNIT as usize
    );

    let cost = execution.happens_before().cost();
    assert_eq!(cost.widest_clock, 1);
    assert_eq!(cost.joins, 0);
    assert_eq!(
        cost.conflict_tests, 1_572_864,
        "with no conflict to cut it short, each event scans every earlier \
         shared event of every other unit: 2048 events cost 768 times as many tests",
    );
}

/// Gated on `debug_assertions`: the guard compiles out under release,
/// where the call answers instead of a panic.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "units_independent pairs two units")]
fn a_unit_paired_with_itself_is_refused() {
    let mut execution = Execution::new();
    execution.push(UnitId::new(0), StepFootprint::default());
    let _ = execution.units_independent(UnitId::new(0), UnitId::new(0));
}
