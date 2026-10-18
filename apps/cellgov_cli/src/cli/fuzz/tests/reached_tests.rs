use super::outcome::*;
use super::outcome_tests::summary_with_findings;

use cellgov_fuzz::FuzzTarget;
use cellgov_ps3_abi::hw::spu;

#[test]
fn a_campaign_names_the_channels_commands_and_events_it_reached() {
    let mut summary = summary_with_findings(0);
    summary
        .spu_facilities
        .channels
        .extend([spu::MFC_CMD, spu::SPU_RD_IN_MBOX]);
    summary
        .spu_facilities
        .mfc_opcodes
        .insert(spu::MFC_GETLLAR as u16);
    let text = render_campaign_summary(
        FuzzTarget::SpuSequence,
        &summary,
        cellgov_fuzz::runner::CampaignOutcome::Clean,
    );
    assert!(
        text.lines()
            .any(|line| line == "fuzz: reached channels=0x15,0x1d mfc_opcodes=0x00d0 events=none"),
        "{text}"
    );
}

#[test]
fn a_campaign_that_reached_no_facility_prints_no_reached_line() {
    let text = render_campaign_summary(
        FuzzTarget::PpuSequence,
        &summary_with_findings(0),
        cellgov_fuzz::runner::CampaignOutcome::Clean,
    );
    assert!(!text.contains("fuzz: reached"), "{text}");
}
