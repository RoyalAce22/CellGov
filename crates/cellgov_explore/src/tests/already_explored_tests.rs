//! The already-explored test Algorithm 2 line 6 asks of a sequence
//! [Abdulla2017 p:42:24 s:Algorithm 2].

use super::*;
use cellgov_mem::{ByteRange, GuestAddr};

/// A step that writes four shared bytes at `start`.
fn store(start: u64) -> StepFootprint {
    StepFootprint {
        shared_writes: vec![ByteRange::new(GuestAddr::new(start), 4).unwrap()],
        ..StepFootprint::default()
    }
}

fn execution(steps: &[(u64, StepFootprint)]) -> Execution {
    let mut execution = Execution::new();
    for (unit, footprint) in steps {
        execution.push(UnitId::new(*unit), footprint.clone());
    }
    execution
}

fn seq_of(execution: &Execution, indices: &[usize]) -> Vec<SeqEvent> {
    indices
        .iter()
        .map(|index| SeqEvent {
            unit: execution.events()[*index].id.unit,
            index: *index,
        })
        .collect()
}

/// Nothing orders any pair.
fn free(_: usize, _: usize) -> bool {
    false
}

#[test]
fn an_empty_sleep_set_explored_nothing() {
    let run = execution(&[(1, store(0))]);
    let sequence = seq_of(&run, &[0]);
    let sleep = BTreeMap::new();
    assert!(!already_explored(&sleep, &sequence, run.events(), &free));
}

#[test]
fn a_sleeping_unit_that_leads_the_sequence_is_already_explored() {
    let run = execution(&[(1, store(0))]);
    let sequence = seq_of(&run, &[0]);
    let sleep = BTreeMap::from([(UnitId::new(1), store(0))]);
    assert!(already_explored(&sleep, &sequence, run.events(), &free));
}

#[test]
fn a_sleeping_unit_whose_step_commutes_past_the_sequence_is_already_explored() {
    let run = execution(&[(1, store(0))]);
    let sequence = seq_of(&run, &[0]);
    let sleep = BTreeMap::from([(UnitId::new(2), StepFootprint::default())]);
    assert!(
        already_explored(&sleep, &sequence, run.events(), &free),
        "unit 2 leads nothing here, but its step commutes past the \
         whole sequence, so the branch through it already covers this",
    );
}

#[test]
fn a_sleeping_unit_whose_step_conflicts_with_the_sequence_explored_nothing() {
    let run = execution(&[(1, store(0))]);
    let sequence = seq_of(&run, &[0]);
    let sleep = BTreeMap::from([(UnitId::new(2), store(0))]);
    assert!(!already_explored(&sleep, &sequence, run.events(), &free));
}

#[test]
fn a_sleeping_unit_the_sequence_holds_back_leads_nothing() {
    let run = execution(&[(1, store(0)), (2, store(0))]);
    let sequence = seq_of(&run, &[0, 1]);
    let held = |first: usize, second: usize| first == 0 && second == 1;
    let sleep = BTreeMap::from([(UnitId::new(2), store(0))]);
    assert!(!already_explored(&sleep, &sequence, run.events(), &held));
}
