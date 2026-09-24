//! The entry points every anchor reader shares: the loader, the
//! stream-level comparison, and the builder `dev record-anchors` files
//! a measurement through.

use cellgov_time::Budget;

use super::super::test_fixtures::{anchor_fixture, test_identity, TEST_CHECKPOINT};
use super::*;

const WITNESSES: &str = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
                         BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n";

/// The stream a run reproducing [`anchor_fixture`]`(73)` prints.
fn matching_stream() -> String {
    let identity = test_identity()
        .render_sentinel_line()
        .expect("fixture identity renders");
    format!("{WITNESSES}{identity}\n")
}

fn run_over(stderr: &str) -> MeasuredRun<'_> {
    MeasuredRun {
        checkpoint: TEST_CHECKPOINT,
        steps: 390099,
        budget: Budget::new(256),
        outcome: BootOutcome::MaxSteps,
        stderr,
    }
}

#[test]
fn a_stream_reproducing_its_anchor_holds() {
    let stream = matching_stream();
    let failures = hold_against_anchor(&anchor_fixture(73), &run_over(&stream));
    assert!(failures.is_empty(), "got {failures:?}");
}

/// The steps and the witnesses come from the measuring child, so the
/// identity triple must come from that same stream.
#[test]
fn a_stream_that_named_no_triple_is_a_disagreement() {
    let failures = hold_against_anchor(&anchor_fixture(73), &run_over(WITNESSES));
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains(RUN_IDENTITY_SENTINEL) && failures[0].contains("no"),
        "got {failures:?}"
    );
}

/// Two identity lines in one stream name no single composition, so the
/// comparison refuses to pick one.
#[test]
fn a_stream_naming_two_triples_is_a_disagreement() {
    let stream = matching_stream();
    let doubled = format!("{stream}{}", stream.lines().last().expect("identity line"));
    let failures = hold_against_anchor(&anchor_fixture(73), &run_over(&doubled));
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains(RUN_IDENTITY_SENTINEL),
        "got {failures:?}"
    );
}

#[test]
fn a_malformed_witness_line_is_a_disagreement() {
    let stream = format!(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=lots\n{}",
        matching_stream()
    );
    let failures = hold_against_anchor(&anchor_fixture(73), &run_over(&stream));
    assert!(
        !failures.is_empty() && failures[0].starts_with("malformed witness line"),
        "got {failures:?}"
    );
}

/// The anchor records the syscall inventory beside the witnesses, so a run
/// that reaches a new unsupported syscall under unmoved counters still
/// fails.
#[test]
fn an_unsupported_syscall_the_anchor_does_not_record_is_a_disagreement() {
    let stream = format!(
        "{}BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=1 999=1@0\n",
        matching_stream()
    );
    let failures = hold_against_anchor(&anchor_fixture(73), &run_over(&stream));
    assert!(
        failures
            .iter()
            .any(|f| f.contains("unsupported syscall inventory")),
        "got {failures:?}"
    );
}

#[test]
fn an_absent_anchor_loads_as_never_recorded() {
    let dir = cellgov_testkit::scratch::scratch();
    let loaded = load_anchor(&dir.join("boot_summary.json")).expect("absent is not an error");
    assert!(loaded.is_none());
}

#[test]
fn a_malformed_anchor_is_an_error_not_an_absence() {
    let dir = cellgov_testkit::scratch::scratch();
    let path = dir.join("boot_summary.json");
    std::fs::write(&path, "{ not json").expect("write scratch anchor");
    let err = load_anchor(&path).expect_err("a damaged anchor must not pass for an absent one");
    assert!(matches!(err, AnchorLoadError::Parse { .. }), "got {err:?}");
    assert!(err.to_string().starts_with("parse "), "got {err}");
}

/// A path that exists but cannot be read as a file is damage, not an
/// unrecorded cell.
#[test]
fn an_unreadable_anchor_is_an_error_not_an_absence() {
    let dir = cellgov_testkit::scratch::scratch();
    let err = load_anchor(&dir).expect_err("a directory is no anchor file");
    assert!(matches!(err, AnchorLoadError::Read { .. }), "got {err:?}");
    assert!(err.to_string().starts_with("read "), "got {err}");
}

#[test]
fn a_committed_anchor_round_trips_through_the_loader() {
    let dir = cellgov_testkit::scratch::scratch();
    let path = dir.join("boot_summary.json");
    let anchor = anchor_fixture(73);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&anchor).expect("serialize"),
    )
    .expect("write scratch anchor");
    assert_eq!(load_anchor(&path).expect("loads"), Some(anchor));
}

/// Re-recording keeps a promoted witness class, so a record run never
/// loosens an exact witness back to an at-least bound.
#[test]
fn a_rebuilt_anchor_keeps_the_previous_witness_classes() {
    let previous = anchor_fixture(73);
    let observed = parse_witness_lines(WITNESSES).expect("witness lines parse");
    let rebuilt = anchor_from_measurement(
        Some(&previous),
        AnchorMeasurement {
            checkpoint: TEST_CHECKPOINT,
            outcome: BootOutcome::MaxSteps,
            steps: 390099,
            budget: Budget::new(256),
            witnesses: observed.values,
            unsupported_syscalls: observed.unsupported_syscalls,
            identity: test_identity(),
        },
    )
    .expect("a valid measurement builds");
    assert_eq!(rebuilt.witnesses, previous.witnesses);
    assert_eq!(rebuilt.host_invariant_breaks, 73);
    assert_eq!(rebuilt.identity, previous.identity);
}
