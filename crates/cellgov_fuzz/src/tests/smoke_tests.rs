use super::*;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::regression;
use crate::seeded::{seed, SeededDefect};
use crate::{FindingKind, RunOutcome};

fn regressions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions")
}

#[test]
fn the_set_names_every_engine_and_strategy_once() {
    let names = SMOKE_CAMPAIGNS
        .iter()
        .map(|campaign| campaign.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), SMOKE_CAMPAIGNS.len());
    let pairs = SMOKE_CAMPAIGNS
        .iter()
        .map(|campaign| (campaign.target, campaign.strategy))
        .collect::<BTreeSet<_>>();
    assert_eq!(pairs.len(), SMOKE_CAMPAIGNS.len());
    for target in [
        FuzzTarget::PpuInstruction,
        FuzzTarget::PpuSequence,
        FuzzTarget::SpuInstruction,
        FuzzTarget::SpuSequence,
    ] {
        for strategy in [GenerationStrategy::Structured, GenerationStrategy::RawWords] {
            assert!(
                pairs.contains(&(target, strategy)),
                "{target:?} {strategy:?}"
            );
        }
    }
    for campaign in &SMOKE_CAMPAIGNS {
        let config = campaign.config();
        config
            .validate_for_target(campaign.target)
            .unwrap_or_else(|error| panic!("{}: {error}", campaign.name));
        assert_eq!(config.schedule.cases.count, campaign.cases);
        assert_eq!(config.seed, campaign.seed);
        assert!(campaign.floor.eligible > 0 && campaign.floor.instruction_kinds > 0);
    }
}

#[test]
fn every_campaign_reaches_its_floor_and_every_finding_is_promoted() {
    let regressions = regression::load(&regressions_dir()).expect("the tracked regressions load");
    for campaign in &SMOKE_CAMPAIGNS {
        let run = campaign.run();
        assert!(
            !matches!(run.outcome, RunOutcome::HarnessFailure(_)),
            "{}: {:?}",
            campaign.name,
            run.outcome
        );
        assert_eq!(run.report.cases, campaign.cases, "{}", campaign.name);
        campaign
            .check_coverage(&run.report)
            .unwrap_or_else(|error| panic!("{error}"));
        for finding in &run.report.findings {
            assert!(
                regression::promoted(&regressions, finding).is_some(),
                "{}: unpromoted finding {:?} {:?} at case {}",
                campaign.name,
                finding.kind,
                finding.fingerprint,
                finding.replay.case_index
            );
        }
        let retained = run.report.findings.len() as u64;
        let counted = run
            .report
            .finding_counts
            .iter()
            .filter(|(kind, _)| !matches!(kind, FindingKind::Unsupported | FindingKind::Undefined))
            .map(|(_, count)| *count)
            .sum::<u64>();
        assert_eq!(
            retained, counted,
            "{}: the budget retains every finding, so every class is seen",
            campaign.name
        );
    }
}

#[test]
fn every_campaign_detects_a_seeded_defect_at_its_budget() {
    // Every engine and strategy checks for a replay that disagrees with its
    // first run, and no campaign of the set reports one otherwise.
    let _guard = seed(SeededDefect::Nondeterministic);
    for campaign in &SMOKE_CAMPAIGNS {
        let run = campaign.run();
        let seeded = run
            .report
            .finding_counts
            .get(&FindingKind::Nondeterministic)
            .copied()
            .unwrap_or(0);
        assert!(
            seeded > 0,
            "{}: the seeded defect went undetected",
            campaign.name
        );
    }
}

#[test]
fn a_run_under_the_floor_is_vacuous() {
    let campaign = SMOKE_CAMPAIGNS[0];
    let mut report = crate::report::FuzzReport::new(
        campaign.target,
        campaign.seed,
        campaign.strategy,
        RetentionConfig::default(),
        4,
        campaign.sequence_words,
    );
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            name: "ppu-instruction-structured",
            metric: "eligible",
            found: 0,
            floor,
        }) if floor == campaign.floor.eligible
    ));
    report.eligible_cases = campaign.floor.eligible;
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_kinds",
            found: 0,
            ..
        })
    ));
}
