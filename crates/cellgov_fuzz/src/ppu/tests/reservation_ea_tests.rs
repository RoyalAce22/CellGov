use super::*;

use crate::campaign::{CampaignSchedule, CaseRange};

#[test]
fn a_store_conditional_past_the_ea_space_is_no_finding() {
    for (target, index) in [
        (FuzzTarget::PpuInstruction, 325_016),
        (FuzzTarget::PpuSequence, 12_922),
    ] {
        let run = target.run(FuzzConfig {
            seed: 11,
            strategy: GenerationStrategy::RawWords,
            schedule: CampaignSchedule {
                cases: CaseRange {
                    first: index,
                    count: 1,
                },
                ..CampaignSchedule::default()
            },
            ..FuzzConfig::default()
        });
        assert_eq!(run.report.cases, 1, "{target:?} case {index}");
        assert!(
            run.report.finding_counts.is_empty(),
            "{target:?} case {index}: {:?}",
            run.report.finding_counts
        );
    }
}
