use cellgov_time::Budget;

use super::super::test_fixtures::{anchor_fixture, test_identity, TEST_CHECKPOINT};
use super::*;

fn observed_stderr(breaks: u64, ldarx: u64) -> ParsedWitnesses {
    parse_witness_lines(&format!(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={breaks}\n\
         BENCH_ATOMIC_WITNESS: ldarx={ldarx} stdcx=0 lwarx=0 stwcx=0\n"
    ))
    .expect("synthetic witness lines parse")
}

/// `anchor_disagreements` against [`anchor_fixture`]`(73)` with every
/// input at its recorded value except those the case overrides.
fn disagreements_with(
    identity: &RunIdentity,
    checkpoint: CheckpointKind,
    steps: u64,
    budget: u64,
    outcome: BootOutcome,
    observed: &ParsedWitnesses,
) -> Vec<String> {
    anchor_disagreements(
        &anchor_fixture(73),
        identity,
        checkpoint,
        steps,
        Budget::new(budget),
        outcome,
        observed,
    )
}

#[test]
fn a_run_matching_its_anchor_reports_no_disagreements() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert!(
        failures.is_empty(),
        "expected no failures, got {failures:?}"
    );
}

#[test]
fn an_exact_witness_that_moved_is_reported() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(77, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("host_invariant_breaks") && failures[0].contains("77"),
        "failure must name the witness and the observed value: {}",
        failures[0]
    );
}

#[test]
fn an_at_least_witness_above_its_baseline_is_not_a_disagreement() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 9_999),
    );
    assert!(failures.is_empty(), "got {failures:?}");
}

#[test]
fn a_moved_step_count_is_reported() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390100,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 390100")),
        "got {failures:?}"
    );
}

#[test]
fn a_changed_outcome_is_reported() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::ProcessExit,
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("outcome ProcessExit")),
        "got {failures:?}"
    );
}

/// A stop condition the run never reaches leaves the step count, the
/// outcome and every witness intact, so nothing else in the comparison
/// sees a moved checkpoint.
#[test]
fn a_run_taken_at_another_checkpoint_is_reported() {
    let failures = disagreements_with(
        &test_identity(),
        CheckpointKind::Pc {
            addr: cellgov_mem::GuestAddr::new(0x1_0000),
        },
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("Pc=0x10000") && failures[0].contains("ProcessExit"),
        "the failure must name both stop conditions: {}",
        failures[0]
    );
}

/// A moved budget retires a different trajectory under a step count
/// that did not move, and the recorded witnesses are at-least bounds
/// that do not catch it.
#[test]
fn a_run_at_another_budget_is_reported() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        512,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("budget 512") && failures[0].contains("256"),
        "got {failures:?}"
    );
}

/// A file copied from another cell reproduces every witness of its own
/// run, so only the embedded identity triple names the wrong cell.
#[test]
fn an_anchor_measured_against_another_firmware_reports_the_triple() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").version = "3.55".to_string();
    let failures = disagreements_with(
        &ran,
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded 4.93") && failures[0].contains("ran 3.55"),
        "got {failures:?}"
    );
}

#[test]
fn an_anchor_measured_against_another_title_version_reports_the_triple() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").version = "update:02.51".to_string();
    let failures = disagreements_with(
        &ran,
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded CG_TEST base")
            && failures[0].contains("ran CG_TEST update:02.51"),
        "got {failures:?}"
    );
}

/// A reinstall from another PUP keeps the console-visible version, so
/// the report carries every compared field.
#[test]
fn two_firmwares_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").pup_sha256 = "ff".to_string();
    let failures = disagreements_with(
        &ran,
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("pup sha256 00") && failures[0].contains("pup sha256 ff"),
        "got {failures:?}"
    );
}

/// The same for the game half: two trees of one version can differ in
/// the `APP_VER` their PARAM.SFO declares.
#[test]
fn two_title_trees_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").app_version =
        Some(crate::identity::AppVersion::AppVer("01.01".to_string()));
    let failures = disagreements_with(
        &ran,
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("app_ver 01.00") && failures[0].contains("app_ver 01.01"),
        "got {failures:?}"
    );
}

/// An anchor can predate the install of one half of the identity
/// triple.
#[test]
fn a_half_the_anchor_never_named_is_reported_as_unidentified() {
    let mut ran = test_identity();
    ran.game = None;
    let failures = disagreements_with(
        &ran,
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(failures[0].contains("(unidentified)"), "got {failures:?}");
}

#[test]
fn a_pc_reached_outcome_at_another_address_is_reported() {
    let mut baseline = anchor_fixture(73);
    baseline.outcome = BootOutcome::PcReached(0x1_0000);
    let observed = observed_stderr(73, 100);
    let at = |pc| {
        anchor_disagreements(
            &baseline,
            &test_identity(),
            TEST_CHECKPOINT,
            390099,
            Budget::new(256),
            BootOutcome::PcReached(pc),
            &observed,
        )
    };
    assert!(at(0x1_0000).is_empty(), "identical outcome must match");
    let moved = at(0x1_0004);
    assert_eq!(moved.len(), 1, "got {moved:?}");
    assert!(
        moved[0].contains("PcReached(0x10004)") && moved[0].contains("PcReached(0x10000)"),
        "the failure must name both outcomes in their Display form: {moved:?}"
    );
}

#[test]
fn an_anchor_with_no_witnesses_is_a_disagreement() {
    let mut baseline = anchor_fixture(73);
    baseline.witnesses.clear();
    let failures = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert_eq!(failures, vec!["anchor records no witnesses".to_string()]);
}

#[test]
fn a_recorded_witness_whose_line_never_appeared_is_reported() {
    let observed = parse_witness_lines("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n")
        .expect("synthetic witness line parses");
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed,
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("ldarx") && f.contains("BENCH_ATOMIC_WITNESS:")),
        "a missing emitter must not read as an observed zero: {failures:?}"
    );
}

#[test]
fn a_witness_the_anchor_does_not_carry_is_reported() {
    let observed = parse_witness_lines(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
         BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n\
         BENCH_DCBZ_WITNESS: count=4\n",
    )
    .expect("synthetic witness lines parse");
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        256,
        BootOutcome::MaxSteps,
        &observed,
    );
    assert_eq!(
        failures,
        vec!["witness dcbz is emitted but not recorded in the anchor".to_string()]
    );
}

#[test]
fn a_zero_step_run_against_a_recorded_anchor_is_a_disagreement() {
    let failures = disagreements_with(
        &test_identity(),
        TEST_CHECKPOINT,
        0,
        256,
        BootOutcome::MaxSteps,
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 0")),
        "got {failures:?}"
    );
}
