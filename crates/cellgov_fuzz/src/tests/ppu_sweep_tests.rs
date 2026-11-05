use super::*;

use std::collections::BTreeSet;

use crate::registry::ppu_descriptors;

#[test]
fn the_sweep_runs_every_ppu_engine_and_strategy_once_past_the_smoke_set() {
    let pairs = PPU_SWEEP
        .iter()
        .map(|campaign| (campaign.target, campaign.strategy))
        .collect::<BTreeSet<_>>();
    assert_eq!(pairs.len(), PPU_SWEEP.len());
    for target in [FuzzTarget::PpuInstruction, FuzzTarget::PpuSequence] {
        for strategy in [GenerationStrategy::Structured, GenerationStrategy::RawWords] {
            assert!(
                pairs.contains(&(target, strategy)),
                "{target:?} {strategy:?}"
            );
        }
    }
    let registry = ppu_descriptors().len() as u64;
    for campaign in &PPU_SWEEP {
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
