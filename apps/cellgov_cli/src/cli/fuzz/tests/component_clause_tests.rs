use super::outcome::*;
use super::outcome_tests::{record, summary_with_findings};

use std::path::PathBuf;

use cellgov_fuzz::artifact::ArtifactReduction;
use cellgov_fuzz::regression::{Regression, RegressionEntry, RegressionProfile, RegressionStatus};
use cellgov_fuzz::FuzzTarget;

fn artifact_in(component: Option<&str>, case: u64) -> ArtifactRecord {
    let mut record = record(true, ArtifactReduction::NotAttempted);
    record.fingerprint.component = component.map(str::to_owned);
    record.case_index = case;
    record.path = PathBuf::from("out").join(format!("{case}.json"));
    record
}

#[test]
fn a_named_component_is_its_own_group_and_prints_after_the_divergence() {
    let mut summary = summary_with_findings(3);
    summary.artifacts = vec![
        artifact_in(Some("Ppu(State)"), 1),
        artifact_in(None, 2),
        artifact_in(Some("Ppu(Memory)"), 3),
    ];
    let text = render_campaign_summary(
        FuzzTarget::PpuInstruction,
        &summary,
        CampaignOutcome::Findings,
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "{text}");
    // Three records that differ only in component are three groups; equal
    // sizes order by component, the unnamed one first.
    let group = |clause: &str, case: u64| {
        format!(
            "fuzz: findings=1 kind=IllegalOutcome check=LegalOutcome divergence=Outcome{clause} instructions=none replay: cellgov dev fuzz replay --artifact {}",
            PathBuf::from("out").join(format!("{case}.json")).display()
        )
    };
    assert_eq!(lines[1], group("", 2));
    assert_eq!(lines[2], group(" component=Ppu(Memory)", 3));
    assert_eq!(lines[3], group(" component=Ppu(State)", 1));
    assert_eq!(
        text,
        render_campaign_summary(
            FuzzTarget::PpuInstruction,
            &summary,
            CampaignOutcome::Findings
        )
    );
}

#[test]
fn the_record_replay_and_promotion_lines_carry_the_component_clause() {
    let named = artifact_in(Some("Ppu(State)"), 1);
    assert_eq!(
        named.render("fuzz: "),
        format!(
            "fuzz: version=3 seed=7 case=1 kind=IllegalOutcome check=LegalOutcome divergence=Outcome component=Ppu(State) reduction=not attempted artifact=stored cellgov dev fuzz replay --artifact {}\n",
            PathBuf::from("out").join("1.json").display()
        )
    );
    let replay = ReplayOutcome {
        case_index: 1,
        reduced: false,
        finding_kind: "IllegalOutcome".into(),
        fingerprint: named.fingerprint.clone(),
        words: vec![0x3860_0006],
    };
    assert_eq!(
        render_replay_outcome(&replay),
        "fuzz replay: reproduced case=1 reduced=false kind=IllegalOutcome check=LegalOutcome divergence=Outcome component=Ppu(State) words=[945815558]\n"
    );
    let mut artifact = super::tests::synthetic_finding_artifact();
    artifact.fingerprint.component = Some("Ppu(State)".into());
    let promotion = Regression {
        entry: RegressionEntry {
            name: "kept".into(),
            status: RegressionStatus::Open,
            profile: RegressionProfile::Both,
            summary: String::new(),
        },
        artifact,
        path: PathBuf::from("kept.json"),
    };
    assert_eq!(
        render_promotion(&promotion),
        "fuzz promote: kept status=Open profile=Both kind=IllegalOutcome check=LegalOutcome divergence=Outcome component=Ppu(State) artifact=kept.json\n"
    );
}
