//! Byte-divergence classification against ELF-header, proc-param, OPD-slot, and sync-id ranges.

use super::*;
use crate::observation::{NamedMemoryRegion, ObservationMetadata, ObservedOutcome};
use strum::VariantArray;

#[test]
fn divergence_class_variants_are_distinct_and_labels_are_unique() {
    let labels: std::collections::BTreeSet<String> = DivergenceClass::VARIANTS
        .iter()
        .map(|c| c.to_string())
        .collect();
    assert_eq!(
        labels.len(),
        DivergenceClass::VARIANTS.len(),
        "DivergenceClass label collision under to_string(): {labels:?}",
    );
}

fn region(name: &str, addr: u64, length: u64) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.to_string(),
        addr,
        data: vec![0u8; length as usize],
    }
}

fn div(offset: u64, length: u64) -> ByteDivergence {
    ByteDivergence {
        offset,
        length,
        a_byte: 0,
        b_byte: 1,
    }
}

fn obs_with(regions: Vec<NamedMemoryRegion>) -> Observation {
    Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov".to_string(),
            steps: Some(1),
        },
        tty_log: Vec::new(),
    }
}

#[test]
fn divergence_class_display_strings_are_stable() {
    assert_eq!(format!("{}", DivergenceClass::ElfHeader), "ElfHeader");
    assert_eq!(format!("{}", DivergenceClass::SysProcParam), "SysProcParam");
    assert_eq!(format!("{}", DivergenceClass::HleOpdSlot), "HleOpdSlot");
    assert_eq!(
        format!("{}", DivergenceClass::SyncPrimitiveId),
        "SyncPrimitiveId",
    );
    assert_eq!(format!("{}", DivergenceClass::Unclassified), "Unclassified");
}

#[test]
fn divergence_class_is_non_semantic_for_four_known_classes() {
    assert!(DivergenceClass::ElfHeader.is_non_semantic());
    assert!(DivergenceClass::SysProcParam.is_non_semantic());
    assert!(DivergenceClass::HleOpdSlot.is_non_semantic());
    assert!(DivergenceClass::SyncPrimitiveId.is_non_semantic());
    assert!(!DivergenceClass::Unclassified.is_non_semantic());
}

#[test]
fn sync_primitive_id_range_classifies_when_populated() {
    let r = region("data", 0x860000, 0x10000);
    let range: Range<u64> = 0x862000..0x862004;
    let ctx = ClassifierContext {
        sync_primitive_id_ranges: vec![range],
        ..ClassifierContext::default()
    };
    assert_eq!(
        classify(&div(0x2000, 4), r.addr, &ctx),
        DivergenceClass::SyncPrimitiveId,
    );
    assert_eq!(
        classify(&div(0x2004, 4), r.addr, &ctx),
        DivergenceClass::Unclassified,
    );
}

#[test]
fn elf_header_offset_inside_range_classifies_as_elf_header() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    let class = classify(&div(0x35, 1), r.addr, &ctx);
    assert_eq!(class, DivergenceClass::ElfHeader);
}

#[test]
fn elf_header_offset_just_past_end_does_not_match() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    let class = classify(&div(0x40, 1), r.addr, &ctx);
    assert_eq!(class, DivergenceClass::Unclassified);
}

#[test]
fn run_straddling_elf_header_boundary_is_unclassified() {
    let r = region("code", 0x10000, 0x80);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    let class = classify(&div(0x38, 0x10), r.addr, &ctx);
    assert_eq!(class, DivergenceClass::Unclassified);
}

#[test]
fn divergence_exactly_filling_range_classifies() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    assert_eq!(
        classify(&div(0x00, 0x40), r.addr, &ctx),
        DivergenceClass::ElfHeader
    );
}

#[test]
fn divergence_at_range_start_classifies() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    assert_eq!(
        classify(&div(0x00, 1), r.addr, &ctx),
        DivergenceClass::ElfHeader
    );
}

#[test]
fn divergence_ending_at_range_end_classifies() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    assert_eq!(
        classify(&div(0x3F, 1), r.addr, &ctx),
        DivergenceClass::ElfHeader
    );
}

#[test]
fn empty_context_returns_unclassified_for_any_divergence() {
    let r = region("code", 0x10000, 0x40);
    let ctx = ClassifierContext::default();
    assert_eq!(
        classify(&div(0x10, 1), r.addr, &ctx),
        DivergenceClass::Unclassified
    );
}

#[test]
fn sys_proc_param_range_classifies_when_populated() {
    let r = region("code", 0x10000, 0x800);
    let ctx = ClassifierContext {
        sys_proc_param_range: Some(0x10700..0x10720),
        ..ClassifierContext::default()
    };
    let class = classify(&div(0x710, 0x10), r.addr, &ctx);
    assert_eq!(class, DivergenceClass::SysProcParam);
}

#[test]
fn hle_opd_range_classifies_when_populated() {
    let r = region("data", 0x820000, 0x10000);
    let opd_range: Range<u64> = 0x824000..0x824400;
    let ctx = ClassifierContext {
        hle_opd_ranges: vec![opd_range],
        ..ClassifierContext::default()
    };
    let class = classify(&div(0x4100, 0x100), r.addr, &ctx);
    assert_eq!(class, DivergenceClass::HleOpdSlot);
}

#[test]
fn hle_opd_ranges_check_every_entry() {
    let r = region("data", 0x820000, 0x10000);
    let ctx = ClassifierContext {
        hle_opd_ranges: vec![0x820000..0x820004, 0x824000..0x824004, 0x828000..0x828004],
        ..ClassifierContext::default()
    };
    assert_eq!(
        classify(&div(0x0, 4), r.addr, &ctx),
        DivergenceClass::HleOpdSlot
    );
    assert_eq!(
        classify(&div(0x4000, 4), r.addr, &ctx),
        DivergenceClass::HleOpdSlot
    );
    assert_eq!(
        classify(&div(0x8000, 4), r.addr, &ctx),
        DivergenceClass::HleOpdSlot
    );
    assert_eq!(
        classify(&div(0x1000, 4), r.addr, &ctx),
        DivergenceClass::Unclassified
    );
}

#[test]
fn from_observation_populates_elf_header_for_code_region() {
    let ctx = ClassifierContext::from_observation(&obs_with(vec![
        region("code", 0x10000, 0x800000),
        region("data", 0x820000, 0x80000),
    ]))
    .expect("well-formed code region must classify");
    assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
    assert!(ctx.sys_proc_param_range.is_none());
    assert!(ctx.hle_opd_ranges.is_empty());
}

#[test]
fn from_observation_picks_first_code_region_when_duplicated() {
    let ctx = ClassifierContext::from_observation(&obs_with(vec![
        region("code", 0x10000, 0x800000),
        region("code", 0x20000, 0x800000),
    ]))
    .expect("first matching code region must classify");
    assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
}

#[test]
fn from_observation_rejects_observation_without_code_region() {
    let err =
        ClassifierContext::from_observation(&obs_with(vec![region("data", 0x820000, 0x80000)]))
            .expect_err("observation without code region must Err");
    assert_eq!(err, ClassifierContextError::MissingCodeRegion);
}

#[test]
fn from_observation_rejects_short_code_region() {
    let err = ClassifierContext::from_observation(&obs_with(vec![region("code", 0x10000, 16)]))
        .expect_err("code region shorter than ELF_HEADER_SIZE must Err");
    assert_eq!(
        err,
        ClassifierContextError::ShortCodeRegion {
            len: 16,
            needed: ELF_HEADER_SIZE,
        }
    );
}

#[test]
fn existing_fixture_offsets_classify_as_elf_header() {
    // 0x35 is the low byte of ELF64 `e_ehsize` (BE).
    // 0x17 is the low byte of ELF64 `e_version` (BE).
    let r = region("code", 0x10000, 0x800000);
    let ctx = ClassifierContext::from_observation(&obs_with(vec![r.clone()]))
        .expect("well-formed code region must classify");
    assert_eq!(
        classify(&div(0x35, 1), r.addr, &ctx),
        DivergenceClass::ElfHeader
    );
    assert_eq!(
        classify(&div(0x17, 1), r.addr, &ctx),
        DivergenceClass::ElfHeader
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "ClassifierContext ranges overlap")]
fn overlapping_ranges_panic_disjoint_check_in_debug() {
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        sys_proc_param_range: Some(0x10020..0x10080),
        hle_opd_ranges: Vec::new(),
        sync_primitive_id_ranges: Vec::new(),
    };
    ctx.debug_assert_disjoint();
}

#[test]
fn abutting_ranges_pass_disjoint_check() {
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        sys_proc_param_range: Some(0x10040..0x10080),
        hle_opd_ranges: Vec::new(),
        sync_primitive_id_ranges: Vec::new(),
    };
    ctx.debug_assert_disjoint();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "is inverted")]
fn inverted_range_panics_disjoint_check_in_debug() {
    #[allow(
        clippy::reversed_empty_ranges,
        reason = "range inversion is the regression under test"
    )]
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10040..0x10000),
        sys_proc_param_range: None,
        hle_opd_ranges: Vec::new(),
        sync_primitive_id_ranges: Vec::new(),
    };
    ctx.debug_assert_disjoint();
}
