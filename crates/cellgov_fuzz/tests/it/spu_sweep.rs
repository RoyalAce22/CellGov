//! The long SPU sweep, run by the scheduled fuzz workflow.

use std::path::Path;

use cellgov_fuzz::regression;
use cellgov_fuzz::spu_sweep::SPU_SWEEP;
use cellgov_fuzz::{FindingKind, RunOutcome};

#[test]
#[ignore = "runs the long SPU sweep; the scheduled fuzz workflow runs it"]
fn the_spu_sweep_reaches_the_smoke_floors_and_every_finding_is_promoted() {
    let regressions = regression::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions"))
        .expect("the tracked regressions load");
    for campaign in &SPU_SWEEP {
        let run = campaign.run();
        assert!(
            !matches!(run.outcome, RunOutcome::HarnessFailure(_)),
            "{}: {:?}",
            campaign.name,
            run.outcome
        );
        assert_eq!(run.report.cases, campaign.cases(), "{}", campaign.name);
        campaign
            .smoke()
            .expect("every sweep has a smoke campaign")
            .check_coverage(&run.report)
            .unwrap_or_else(|error| panic!("{}: {error}", campaign.name));
        for finding in &run.report.findings {
            assert!(
                regression::promoted(&regressions, finding).is_some(),
                "{}: unpromoted finding {:?} {:?}, replay {:?}",
                campaign.name,
                finding.kind,
                finding.fingerprint,
                finding.replay
            );
        }
        let counted: u64 = run
            .report
            .finding_counts
            .iter()
            .filter(|(kind, _)| !matches!(kind, FindingKind::Unsupported | FindingKind::Undefined))
            .map(|(_, count)| *count)
            .sum();
        assert_eq!(
            run.report.findings.len() as u64,
            counted,
            "{}: the sweep retains every finding it counts",
            campaign.name
        );
    }
}
