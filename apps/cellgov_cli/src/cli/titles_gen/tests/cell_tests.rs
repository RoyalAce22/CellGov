//! Which token each combination of expectation and committed
//! artifacts renders as.

use cellgov_compare::BootOutcome;

use super::super::test_fixtures::*;
use super::*;

fn cell(expect: CellExpectation) -> MatrixCell {
    let mut c = matrix_cell(reference_key());
    c.expect = expect;
    c
}

fn classify(expect: CellExpectation, artifacts: CellArtifacts) -> String {
    CellResult::classify(&cell(expect), &artifacts).token()
}

fn with_cross(summary: cellgov_compare::CrossRunnerSummary) -> CellArtifacts {
    CellArtifacts {
        boot: None,
        cross: Some(summary),
    }
}

fn with_anchor(outcome: BootOutcome) -> CellArtifacts {
    CellArtifacts {
        boot: Some(boot(outcome, 1_000)),
        cross: None,
    }
}

#[test]
fn a_byte_equivalent_convergence_is_ok() {
    assert_eq!(
        classify(CellExpectation::Frontier, with_cross(converged(0))),
        "ok"
    );
}

#[test]
fn classified_divergent_bytes_are_still_ok() {
    assert_eq!(
        classify(CellExpectation::Frontier, with_cross(converged(975))),
        "ok"
    );
}

#[test]
fn unclassified_bytes_render_pending() {
    assert_eq!(
        classify(
            CellExpectation::Frontier,
            with_cross(converged_pending(666, 56))
        ),
        "pending"
    );
}

#[test]
fn a_frontier_cells_divergence_names_the_reason() {
    assert_eq!(
        classify(CellExpectation::Frontier, with_cross(diverged())),
        format!("frontier ({DIVERGED_REASON})")
    );
}

#[test]
fn a_probe_cells_divergence_is_never_the_frontier_map() {
    let token = classify(CellExpectation::Probe, with_cross(diverged()));
    assert_eq!(token, format!("probe ({DIVERGED_REASON})"));
    assert!(!token.contains("frontier"), "{token}");
}

#[test]
fn a_probe_cell_that_converged_renders_distinctly() {
    assert_eq!(
        classify(CellExpectation::Probe, with_cross(converged(0))),
        "probe (converged unexpectedly)"
    );
}

#[test]
fn an_anchor_alone_names_its_outcome_and_claims_no_verdict() {
    assert_eq!(
        classify(CellExpectation::Frontier, with_anchor(BootOutcome::Fault)),
        "anchor (Fault)"
    );
}

#[test]
fn a_probe_cells_anchor_is_the_error_the_guest_got() {
    assert_eq!(
        classify(CellExpectation::Probe, with_anchor(BootOutcome::Fault)),
        "probe (Fault)"
    );
}

#[test]
fn a_declared_cell_with_nothing_recorded_is_a_dot() {
    assert_eq!(
        classify(CellExpectation::Frontier, CellArtifacts::default()),
        "."
    );
}

#[test]
fn a_pending_reason_renders_beside_the_dot() {
    let mut c = cell(CellExpectation::Frontier);
    c.pending = Some("the firmware is not obtainable".to_string());
    assert_eq!(
        CellResult::classify(&c, &CellArtifacts::default()).token(),
        ". (the firmware is not obtainable)"
    );
}

#[test]
fn a_cross_runner_verdict_outranks_the_anchor_beside_it() {
    let artifacts = CellArtifacts {
        boot: Some(boot(BootOutcome::ProcessExit, 11_212)),
        cross: Some(diverged()),
    };
    assert_eq!(
        CellResult::classify(&cell(CellExpectation::Frontier), &artifacts).token(),
        format!("frontier ({DIVERGED_REASON})")
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "markdown-table-breaking")]
fn a_reason_carrying_a_table_separator_is_caught_before_it_reaches_the_grid() {
    CellResult::Frontier {
        reason: "region a | region b".to_string(),
    }
    .token();
}

#[test]
fn only_an_unrecorded_cell_is_uncounted_by_coverage() {
    let recorded = [
        CellResult::Ok,
        CellResult::Pending,
        CellResult::Frontier {
            reason: DIVERGED_REASON.to_string(),
        },
        CellResult::Probe {
            observed: "Fault".to_string(),
        },
        CellResult::ProbeConverged,
        CellResult::AnchorOnly {
            outcome: "MaxSteps".to_string(),
        },
    ];
    assert!(recorded.iter().all(CellResult::is_recorded));
    assert!(!CellResult::Unrecorded { reason: None }.is_recorded());
    assert!(!CellResult::Unrecorded {
        reason: Some("stated".to_string())
    }
    .is_recorded());
}
