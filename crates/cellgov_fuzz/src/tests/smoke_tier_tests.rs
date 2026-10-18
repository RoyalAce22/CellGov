//! Each SPU tier of the smoke set reaches its floor, fails it when a
//! metric drops, and catches a defect seeded into it at its budget.

use super::*;

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::report::FindingKind;
use crate::seeded::{seed, SeededDefect};
use crate::spu_reference::SpuReferenceFileOutcome;

fn smoke(name: &str) -> SmokeCampaign {
    *SMOKE_CAMPAIGNS
        .iter()
        .find(|campaign| campaign.name == name)
        .unwrap_or_else(|| panic!("the smoke set has {name}"))
}

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spu_reference")
}

fn empty_report(campaign: &SmokeCampaign) -> FuzzReport {
    FuzzReport::new(
        campaign.target,
        campaign.seed,
        campaign.strategy,
        RetentionConfig::default(),
        4,
        campaign.sequence_words,
    )
}

#[test]
fn a_registry_the_floor_was_not_measured_against_fails_the_set() {
    for campaign in &SMOKE_CAMPAIGNS {
        let mut stale = *campaign;
        stale.floor.registry_kinds += 1;
        assert!(
            matches!(
                stale.check_coverage(&empty_report(campaign)),
                Err(SmokeError::Unpinned { pinned, registry, .. })
                    if pinned == registry + 1 && registry == campaign.floor.registry_kinds
            ),
            "{}",
            campaign.name
        );
    }
    let replay = SMOKE_REFERENCE
        .run(&reference_dir())
        .expect("the fixtures read");
    let mut stale = SMOKE_REFERENCE;
    stale.units -= 1;
    assert!(matches!(
        stale.check(&replay),
        Err(SmokeError::Unpinned {
            name: "spu-reference-replay",
            pinned,
            registry,
        }) if pinned + 1 == registry
    ));
}

#[test]
fn the_structured_instruction_campaign_needs_every_kind_and_every_declared_relation() {
    assert_eq!(
        crate::spu::declared_relation_checks(),
        BTreeSet::from([
            CheckIdentity::SpuIgnoredField,
            CheckIdentity::SpuCountMasking,
            CheckIdentity::SpuShufbControlClass,
            CheckIdentity::SpuImmediateRegister,
            CheckIdentity::SpuCommutative,
            CheckIdentity::SpuSlotPermutation,
            CheckIdentity::SpuCompareBranch,
        ])
    );
    let campaign = smoke("spu-instruction-structured");
    let mut report = campaign.run().report;
    campaign
        .check_coverage(&report)
        .unwrap_or_else(|error| panic!("{error}"));
    let kind = *report.instruction_kinds.first().expect("a kind ran");
    report.instruction_kinds.remove(&kind);
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_kinds",
            found,
            floor,
            ..
        }) if found + 1 == floor && floor == SPU_REGISTRY_KINDS
    ));
    // A kind outside the registry does not count toward the registry's kinds.
    report
        .instruction_kinds
        .insert(InstructionIdentity::Ppu(ppu_descriptors()[0].kind));
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_kinds",
            ..
        })
    ));
    report.instruction_kinds.insert(kind);
    report
        .metamorphic_executions
        .remove(&CheckIdentity::SpuShufbControlClass);
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_relations",
            found: 6,
            floor: 7,
            ..
        })
    ));
}

#[test]
fn a_sequence_campaign_fails_its_floor_when_a_facility_goes_unreached() {
    let campaign = smoke("spu-sequence-structured");
    let report = campaign.run().report;
    campaign
        .check_coverage(&report)
        .unwrap_or_else(|error| panic!("{error}"));
    let facilities = &report.spu_facilities;
    // The world serviced a get, a put and both atomic commands, and the
    // signal and mailbox events went pending.
    for opcode in [0x20, 0x40, 0xb4, 0xd0] {
        assert!(facilities.mfc_opcodes.contains(&opcode), "{opcode:#x}");
    }
    for event in [
        cellgov_ps3_abi::hw::spu::event::S1,
        cellgov_ps3_abi::hw::spu::event::MB,
    ] {
        assert!(facilities.event_sources.contains(&event), "{event:#x}");
    }
    let mut channels = report.clone();
    channels.spu_facilities.channels.pop_last();
    let mut opcodes = report.clone();
    opcodes.spu_facilities.mfc_opcodes.pop_last();
    let mut events = report.clone();
    events.spu_facilities.event_sources.pop_last();
    for (metric, stripped) in [
        ("channels", channels),
        ("mfc_opcodes", opcodes),
        ("event_sources", events),
    ] {
        assert!(
            matches!(
                campaign.check_coverage(&stripped),
                Err(SmokeError::Vacuous { metric: found_metric, found, floor, .. })
                    if found_metric == metric && found + 1 == floor
            ),
            "{metric}"
        );
    }
}

#[test]
fn the_reference_tier_replays_every_committed_fixture_to_its_floor() {
    let replay = SMOKE_REFERENCE
        .run(&reference_dir())
        .expect("the fixtures read");
    SMOKE_REFERENCE
        .check(&replay)
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(replay.files.len() as u64, SMOKE_REFERENCE.files);
    assert_eq!(replay.vectors() as u64, SMOKE_REFERENCE.vectors);
    assert!(SMOKE_REFERENCE.reaches_floor(&replay));
    let mut fewer = replay;
    let dropped = fewer.files.pop().expect("a file");
    assert!(!SMOKE_REFERENCE.reaches_floor(&fewer));
    assert!(matches!(
        SMOKE_REFERENCE.check(&fewer),
        Err(SmokeError::Vacuous {
            metric: "files",
            ..
        })
    ));
    fewer.files.push(dropped);
    let vectors = fewer
        .files
        .iter_mut()
        .find_map(|run| match &mut run.outcome {
            SpuReferenceFileOutcome::Set(replays) if replays.len() > 1 => Some(replays),
            _ => None,
        })
        .expect("a file holds a vector set");
    vectors.pop();
    assert!(!SMOKE_REFERENCE.reaches_floor(&fewer));
    assert!(matches!(
        SMOKE_REFERENCE.check(&fewer),
        Err(SmokeError::Vacuous {
            metric: "vectors",
            found,
            floor,
            ..
        }) if found + 1 == floor
    ));
    fewer.completeness.missing.push("instruction:a".into());
    assert!(matches!(
        SMOKE_REFERENCE.check(&fewer),
        Err(SmokeError::ReferenceIncomplete { gaps: 1, .. })
    ));
    fewer.completeness.missing.clear();
    fewer.completeness.unowned.push("orphan.json".into());
    assert!(matches!(
        SMOKE_REFERENCE.check(&fewer),
        Err(SmokeError::ReferenceIncomplete { gaps: 1, .. })
    ));
}

#[test]
fn a_misplaced_get_byte_is_caught_by_the_reference_tier_and_no_self_check() {
    let _guard = seed(SeededDefect::MisLandedGet);
    let replay = SMOKE_REFERENCE
        .run(&reference_dir())
        .expect("the fixtures read");
    assert!(matches!(
        SMOKE_REFERENCE.check(&replay),
        Err(SmokeError::ReferenceMismatch { .. })
    ));
    // Both runs of a sequence land the same wrong byte, so the replay
    // agrees with itself.
    for name in ["spu-sequence-structured", "spu-sequence-raw-words"] {
        let run = smoke(name).run();
        assert!(
            run.report.is_clean(),
            "{name}: {:?}",
            run.report.finding_counts
        );
    }
    assert!(smoke("spu-sequence-structured")
        .run()
        .report
        .spu_facilities
        .mfc_opcodes
        .contains(&0x40));
}

#[test]
fn a_late_world_is_caught_by_the_serviced_sequence_campaign_at_its_budget() {
    let _guard = seed(SeededDefect::LateWorld);
    let run = smoke("spu-sequence-structured").run();
    assert!(
        run.report
            .finding_counts
            .get(&FindingKind::Nondeterministic)
            .is_some_and(|&count| count > 0),
        "the late world went undetected"
    );
    assert_eq!(
        run.report.finding_counts.keys().collect::<Vec<_>>(),
        [&FindingKind::Nondeterministic]
    );
    // The instruction tiers run no world, so they cannot see it.
    for name in ["spu-instruction-structured", "spu-instruction-raw-words"] {
        assert!(smoke(name).run().report.is_clean(), "{name}");
    }
}

#[test]
fn each_relation_defect_is_caught_by_the_structured_instruction_campaign_at_its_budget() {
    let campaign = smoke("spu-instruction-structured");
    for (defect, check) in [
        (
            SeededDefect::IgnoredFieldRead,
            CheckIdentity::SpuIgnoredField,
        ),
        (SeededDefect::UnmaskedCount, CheckIdentity::SpuCountMasking),
        (
            SeededDefect::ImmediateFormOnly,
            CheckIdentity::SpuImmediateRegister,
        ),
        (SeededDefect::OperandOrder, CheckIdentity::SpuCommutative),
        (SeededDefect::FirstSlot, CheckIdentity::SpuSlotPermutation),
        (
            SeededDefect::BranchFallThrough,
            CheckIdentity::SpuCompareBranch,
        ),
    ] {
        let _guard = seed(defect);
        let report = campaign.run().report;
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.fingerprint.check == check),
            "{defect:?}: {check:?} did not catch it"
        );
        // A defect may break a second relation too, as the rotqbyi count
        // does its register form; it breaks no other kind of check.
        assert_eq!(
            report.finding_counts.keys().collect::<Vec<_>>(),
            [&FindingKind::MetamorphicViolation],
            "{defect:?}"
        );
    }
}
