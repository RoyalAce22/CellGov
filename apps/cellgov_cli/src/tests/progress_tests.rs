//! The phase enums and the label tables index each other; nothing but
//! a test keeps them aligned.

use super::*;

use cellgov_terminal::progress::Unit;

const ALL_BOOT: [BootPhase; 2] = [BootPhase::Loading, BootPhase::Stepping];

/// The label each variant must carry.
///
/// Exhaustive by construction: a variant added to [`BootPhase`] fails
/// to compile until it is answered here.
fn expected_boot_label(phase: BootPhase) -> &'static str {
    match phase {
        BootPhase::Loading => "loading",
        BootPhase::Stepping => "stepping",
    }
}

#[test]
fn every_boot_phase_indexes_its_own_label_in_both_boot_tasks() {
    for task in [&BENCH_TASK, &RUN_TASK] {
        assert_eq!(task.phases.len(), ALL_BOOT.len(), "{}", task.tag);
        for phase in ALL_BOOT {
            assert_eq!(
                task.phases.get(phase.code() as usize).copied(),
                Some(expected_boot_label(phase)),
                "{} {phase:?}",
                task.tag
            );
        }
    }
}

#[test]
fn the_measured_boot_phase_is_the_step_loop() {
    for task in [&BENCH_TASK, &RUN_TASK] {
        assert_eq!(task.measured, BootPhase::Stepping.code(), "{}", task.tag);
        assert_eq!(task.unit, Unit::Steps, "{}", task.tag);
        assert!(
            (task.measured as usize) < task.phases.len(),
            "{}: the measured phase must name a label",
            task.tag
        );
    }
}

const ALL_PAIR: [BenchPairPhase; 2] = [BenchPairPhase::Measuring, BenchPairPhase::Comparing];

/// Exhaustive by construction, like [`expected_boot_label`].
fn expected_pair_label(phase: BenchPairPhase) -> &'static str {
    match phase {
        BenchPairPhase::Measuring => "measuring",
        BenchPairPhase::Comparing => "comparing",
    }
}

#[test]
fn every_pair_phase_indexes_its_own_label() {
    assert_eq!(BENCH_PAIR_TASK.phases.len(), ALL_PAIR.len());
    for phase in ALL_PAIR {
        assert_eq!(
            BENCH_PAIR_TASK.phases.get(phase.code() as usize).copied(),
            Some(expected_pair_label(phase)),
            "{phase:?}"
        );
    }
    assert_eq!(BENCH_PAIR_TASK.measured, BenchPairPhase::Measuring.code());
    assert_eq!(BENCH_PAIR_TASK.unit, Unit::Items);
}

#[test]
fn the_anchor_task_measures_the_one_phase_it_has() {
    assert_eq!(RECORD_ANCHORS_TASK.phases.len(), 1);
    assert!((RECORD_ANCHORS_TASK.measured as usize) < RECORD_ANCHORS_TASK.phases.len());
    assert_eq!(RECORD_ANCHORS_TASK.unit, Unit::Items);
}

/// A task streams exactly when its command writes lines while the bar
/// is up. An in-place frame under a streaming writer cursor-ups over
/// lines that have already scrolled.
#[test]
fn a_task_streams_exactly_when_its_command_prints_while_working() {
    for (task, streams) in [
        // Guest TTY and `--trace` lines run the whole step loop.
        (&RUN_TASK, true),
        // Each run's result line prints as that run lands.
        (&BENCH_PAIR_TASK, true),
        // One verdict line per title.
        (&RECORD_ANCHORS_TASK, true),
        // The PRX loader and every module_start print through the load
        // phase, and a child's init pass prints from inside the step
        // loop.
        (&BENCH_TASK, true),
    ] {
        assert_eq!(task.streaming, streams, "{}", task.tag);
    }
}

#[test]
fn every_task_string_is_ascii_so_a_byte_budget_is_a_column_budget() {
    for task in [
        &BENCH_TASK,
        &RUN_TASK,
        &BENCH_PAIR_TASK,
        &RECORD_ANCHORS_TASK,
    ] {
        assert!(task.verb.is_ascii(), "{}", task.tag);
        assert!(task.tag.is_ascii(), "{}", task.tag);
        assert!(task.items.is_ascii(), "{}", task.tag);
        for label in task.phases {
            assert!(label.is_ascii(), "{} {label}", task.tag);
        }
    }
}

/// The 40-column floor. Line 1 spends:
///
/// - the verb,
/// - one space,
/// - the widest status, in brackets, with two separating columns.
///
/// What is left must still tell one title from another.
#[test]
fn every_task_leaves_label_room_at_the_forty_column_floor() {
    /// `compose_frame`'s line-1 budget: the space after the verb, plus
    /// the `  [` and `]` around the status.
    const LINE1_FIXED: usize = 5;
    /// Enough columns to tell two content ids apart.
    const LABEL_FLOOR: usize = 8;
    for task in [
        &BENCH_TASK,
        &RUN_TASK,
        &BENCH_PAIR_TASK,
        &RECORD_ANCHORS_TASK,
    ] {
        let widest = task
            .phases
            .iter()
            .map(|l| l.len())
            .max()
            .expect("every task declares at least one phase");
        let spent = task.verb.len() + LINE1_FIXED + widest;
        assert!(
            spent + LABEL_FLOOR <= 40,
            "{}: verb + status spends {spent} of 40 columns, leaving under {LABEL_FLOOR} \
             for the label",
            task.tag
        );
    }
}
