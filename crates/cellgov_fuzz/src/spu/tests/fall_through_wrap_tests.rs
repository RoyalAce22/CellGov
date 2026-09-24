use crate::campaign::{CampaignSchedule, CaseRange};
use crate::{FuzzConfig, FuzzTarget, GenerationStrategy};

#[test]
fn a_sequence_that_falls_off_the_last_local_store_word_is_no_finding() {
    for index in [1_019_633, 1_293_148, 1_703_325] {
        let run = FuzzTarget::SpuSequence.run(FuzzConfig {
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
        assert_eq!(run.report.cases, 1, "case {index}");
        assert!(
            run.report.finding_counts.is_empty(),
            "case {index}: {:?}",
            run.report.finding_counts
        );
    }
}
