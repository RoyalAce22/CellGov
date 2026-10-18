use cellgov_effects::EffectKind;
use cellgov_ps3_abi::hw::spu;

use super::*;
use crate::{CampaignSchedule, CampaignShard, CaseRange};

fn structured(cases: u64, sequence_words: u32) -> FuzzConfig {
    FuzzConfig {
        seed: 11,
        strategy: GenerationStrategy::Structured,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: cases,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        },
        max_findings: 16,
        sequence_words,
        ..FuzzConfig::default()
    }
}

#[test]
fn a_structured_campaign_reaches_each_facility_pattern_and_finds_nothing() {
    let run = run_sequences(structured(512, 6));
    let report = &run.report;
    assert!(report.is_clean(), "{:?}", report.finding_counts);
    let facilities = &report.spu_facilities;
    for opcode in [
        spu::MFC_GET,
        spu::MFC_PUT,
        spu::MFC_GETLLAR,
        spu::MFC_PUTLLC,
    ] {
        assert!(
            facilities.mfc_opcodes.contains(&(opcode as u16)),
            "opcode {opcode:#x}: {:?}",
            facilities.mfc_opcodes
        );
    }
    for channel in [
        spu::SPU_RD_EVENT_STAT,
        spu::SPU_WR_EVENT_MASK,
        spu::SPU_WR_EVENT_ACK,
        spu::MFC_LSA,
        spu::MFC_CMD,
        spu::MFC_WR_TAG_MASK,
        spu::MFC_WR_TAG_UPDATE,
        spu::MFC_RD_TAG_STAT,
        spu::MFC_RD_ATOMIC_STAT,
        spu::SPU_WR_OUT_MBOX,
        spu::SPU_RD_IN_MBOX,
    ] {
        assert!(
            facilities.channels.contains(&channel),
            "channel {channel}: {:?}",
            facilities.channels
        );
    }
    assert!(
        facilities.event_sources.contains(&spu::event::S1),
        "{:?}",
        facilities.event_sources
    );
    // The campaign sees the effects of the steps after the first yield:
    // a putllc comes after the getllar's read.
    for kind in [EffectKind::DmaEnqueue, EffectKind::ConditionalStore] {
        assert!(
            report.effect_classes.contains_key(&kind),
            "{kind:?}: {:?}",
            report.effect_classes
        );
    }
}

#[test]
fn a_facility_campaign_replays_to_the_same_report() {
    let first = run_sequences(structured(64, 6));
    let second = run_sequences(structured(64, 6));
    assert_eq!(first.report.spu_facilities, second.report.spu_facilities);
    assert_eq!(first.report.effect_classes, second.report.effect_classes);
    assert_eq!(first.report.finding_counts, second.report.finding_counts);
}
