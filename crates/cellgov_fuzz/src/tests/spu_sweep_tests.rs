use super::*;

use std::collections::BTreeSet;

#[test]
fn the_sweep_runs_every_spu_engine_and_strategy_once_past_the_smoke_set() {
    let pairs = SPU_SWEEP
        .iter()
        .map(|campaign| (campaign.target, campaign.strategy))
        .collect::<BTreeSet<_>>();
    assert_eq!(pairs.len(), SPU_SWEEP.len());
    for target in [FuzzTarget::SpuInstruction, FuzzTarget::SpuSequence] {
        for strategy in [GenerationStrategy::Structured, GenerationStrategy::RawWords] {
            assert!(
                pairs.contains(&(target, strategy)),
                "{target:?} {strategy:?}"
            );
        }
    }
    let registry = spu_descriptors().len() as u64;
    for campaign in &SPU_SWEEP {
        let config = campaign.config();
        config
            .validate_for_target(campaign.target)
            .unwrap_or_else(|error| panic!("{}: {error}", campaign.name));
        assert_eq!(
            config.schedule.cases.count,
            registry * campaign.cases_per_kind
        );
        let smoke = campaign.smoke().expect("every sweep has a smoke campaign");
        assert_ne!(campaign.seed, smoke.seed, "{}", campaign.name);
        assert!(
            campaign.cases() >= 10 * smoke.cases,
            "{}: a sweep runs ten times its smoke budget or more",
            campaign.name
        );
    }
}
