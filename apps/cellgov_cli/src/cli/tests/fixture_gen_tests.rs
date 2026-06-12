//! Fixture-gen template substitution, ELF header-range parsing, and classifier-context construction.

use super::*;
use cellgov_compare::{
    compare_observations, NamedMemoryRegion, ObservationMetadata, ObservedOutcome,
};

fn obs(outcome: ObservedOutcome, regions: Vec<NamedMemoryRegion>) -> Observation {
    Observation {
        outcome,
        memory_regions: regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "test".to_string(),
            steps: Some(1),
        },
        tty_log: Vec::new(),
    }
}

fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.to_string(),
        addr,
        data,
    }
}

fn synthetic_elf64_be(phoff: u64, phentsize: u16, phnum: u16) -> Vec<u8> {
    let mut eboot = vec![0u8; 64];
    eboot[0..4].copy_from_slice(b"\x7fELF");
    eboot[4] = 2; // ELFCLASS64
    eboot[5] = 2; // ELFDATA2MSB
    eboot[32..40].copy_from_slice(&phoff.to_be_bytes());
    eboot[54..56].copy_from_slice(&phentsize.to_be_bytes());
    eboot[56..58].copy_from_slice(&phnum.to_be_bytes());
    eboot
}

#[test]
fn apply_subs_replaces_named_tokens() {
    let out = apply_subs(
        "Hello {{name}} -- you are {{role}}",
        &[("name", "World"), ("role", "tester")],
    );
    assert_eq!(out, "Hello World -- you are tester");
}

#[test]
fn apply_subs_leaves_unknown_tokens_visible() {
    let out = apply_subs("Stale: {{ghost}}", &[("ignored", "value")]);
    assert_eq!(out, "Stale: {{ghost}}");
}

#[test]
fn apply_subs_is_deterministic_regardless_of_slice_order() {
    let a = apply_subs("{{a}}/{{b}}", &[("a", "1"), ("b", "2")]);
    let b = apply_subs("{{a}}/{{b}}", &[("b", "2"), ("a", "1")]);
    assert_eq!(a, b);
}

#[test]
fn apply_subs_does_not_re_substitute_into_value() {
    let out = apply_subs("[{{a}}]", &[("a", "{{b}}"), ("b", "EVIL")]);
    assert_eq!(out, "[{{b}}]");
}

#[test]
fn apply_subs_handles_unterminated_token() {
    let out = apply_subs("trailing {{open", &[("open", "X")]);
    assert_eq!(out, "trailing {{open");
}

#[test]
fn build_classifier_context_populates_elf_header_when_code_region_present() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    let eboot = synthetic_elf64_be(0, 0, 0);
    let ctx = build_classifier_context(&eboot, &observation).unwrap();
    assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
}

#[test]
fn elf_header_range_widens_to_include_phdr_table() {
    // phoff=0x40 + 5 * phentsize=0x38 -> PHDR end at 0x158.
    let eboot = synthetic_elf64_be(0x40, 0x38, 5);
    assert_eq!(elf_header_plus_phdr_table_end(&eboot).unwrap(), 0x158);
}

#[test]
fn elf_header_plus_phdr_helper_rejects_short_input() {
    assert!(matches!(
        elf_header_plus_phdr_table_end(&[0u8; 32]),
        Err(ElfHeaderParseError::TooShort { len: 32 })
    ));
}

#[test]
fn elf_header_plus_phdr_helper_rejects_bad_magic() {
    let mut eboot = synthetic_elf64_be(0, 0, 0);
    eboot[0] = 0xCC;
    assert!(matches!(
        elf_header_plus_phdr_table_end(&eboot),
        Err(ElfHeaderParseError::BadMagic { .. })
    ));
}

#[test]
fn elf_header_plus_phdr_helper_rejects_elf_class_32() {
    let mut eboot = synthetic_elf64_be(0, 0, 0);
    eboot[4] = 1; // ELFCLASS32
    assert!(matches!(
        elf_header_plus_phdr_table_end(&eboot),
        Err(ElfHeaderParseError::WrongClass { found: 1 })
    ));
}

#[test]
fn elf_header_plus_phdr_helper_rejects_little_endian() {
    let mut eboot = synthetic_elf64_be(0, 0, 0);
    eboot[5] = 1; // ELFDATA2LSB
    assert!(matches!(
        elf_header_plus_phdr_table_end(&eboot),
        Err(ElfHeaderParseError::WrongEndian { found: 1 })
    ));
}

#[test]
fn elf_header_plus_phdr_helper_rejects_phdr_overflow() {
    let eboot = synthetic_elf64_be(u64::MAX, u16::MAX, u16::MAX);
    assert!(matches!(
        elf_header_plus_phdr_table_end(&eboot),
        Err(ElfHeaderParseError::PhdrTableOverflow { .. })
    ));
}

#[test]
fn build_classifier_context_with_no_code_region_leaves_header_none() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0u8; 4])],
    );
    let eboot = synthetic_elf64_be(0, 0, 0);
    let ctx = build_classifier_context(&eboot, &observation).unwrap();
    assert!(ctx.elf_header_range.is_none());
}

#[test]
fn build_classifier_context_propagates_elf_parse_error() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    let eboot = vec![0u8; 32];
    assert!(matches!(
        build_classifier_context(&eboot, &observation),
        Err(FixtureGenError::ElfHeaderParse(
            ElfHeaderParseError::TooShort { len: 32 }
        ))
    ));
}

#[test]
fn compute_hle_opd_ranges_no_imports_table_is_empty_vec() {
    let eboot = synthetic_elf64_be(0, 0, 0);
    assert_eq!(
        compute_hle_opd_ranges(&eboot).unwrap(),
        Vec::<Range<u64>>::new()
    );
}

#[test]
fn compute_hle_opd_ranges_propagates_non_no_imports_table_errors() {
    let eboot = vec![0u8; 32];
    match compute_hle_opd_ranges(&eboot) {
        Err(FixtureGenError::ImportParse(e)) => assert!(
            !matches!(e, cellgov_ppu::prx::ImportParseError::NoImportsTable),
            "NoImportsTable must be mapped to Ok(vec![]); got Err propagation"
        ),
        other => panic!("expected ImportParse error, got {other:?}"),
    }
}

#[test]
fn merge_adjacent_stub_ranges_empty_input_returns_empty() {
    let mut stubs: Vec<u32> = vec![];
    assert!(merge_adjacent_stub_ranges(&mut stubs).is_empty());
}

#[test]
fn merge_adjacent_stub_ranges_single_stub_one_range() {
    let mut stubs = vec![0x10_0000u32];
    let ranges = merge_adjacent_stub_ranges(&mut stubs);
    assert_eq!(ranges, vec![0x10_0000u64..0x10_0004u64]);
}

#[test]
fn merge_adjacent_stub_ranges_two_adjacent_merge_to_one() {
    let mut stubs = vec![0x10_0000u32, 0x10_0004u32];
    let ranges = merge_adjacent_stub_ranges(&mut stubs);
    assert_eq!(ranges, vec![0x10_0000u64..0x10_0008u64]);
}

#[test]
fn merge_adjacent_stub_ranges_two_non_adjacent_stay_two() {
    let mut stubs = vec![0x10_0000u32, 0x10_0010u32];
    let ranges = merge_adjacent_stub_ranges(&mut stubs);
    assert_eq!(
        ranges,
        vec![0x10_0000u64..0x10_0004u64, 0x10_0010u64..0x10_0014u64]
    );
}

#[test]
fn merge_adjacent_stub_ranges_unsorted_with_dupes_sorts_and_dedups() {
    let mut stubs = vec![
        0x10_0008u32,
        0x10_0000u32,
        0x10_0004u32,
        0x10_0000u32,
        0x10_0010u32,
    ];
    let ranges = merge_adjacent_stub_ranges(&mut stubs);
    assert_eq!(
        ranges,
        vec![0x10_0000u64..0x10_000Cu64, 0x10_0010u64..0x10_0014u64]
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "overlap")]
fn classifier_context_overlap_panics_in_debug() {
    let ctx = ClassifierContext {
        elf_header_range: Some(0x1000..0x2000),
        sys_proc_param_range: Some(0x1500..0x2500),
        hle_opd_ranges: Vec::new(),
        sync_primitive_id_ranges: Vec::new(),
    };
    ctx.debug_assert_disjoint();
}

#[test]
fn build_classifier_context_overflows_on_code_region_addr_near_u64_max() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", u64::MAX - 0x20, vec![0u8; 0x40])],
    );
    // phoff=0x40 + 5 * 0x38 = 0x158 PHDR end; adds to addr -> overflow.
    let eboot = synthetic_elf64_be(0x40, 0x38, 5);
    assert!(matches!(
        build_classifier_context(&eboot, &observation),
        Err(FixtureGenError::CodeRegionAddrOverflow { .. })
    ));
}

/// Synthetic EBOOT with a single PT_LOAD covering a
/// sys_proc_param magic struct at file offset 0x100.
fn synthetic_eboot_with_sys_proc_param_at(p_vaddr: u64, struct_size: u32) -> Vec<u8> {
    use cellgov_ps3_abi::elf::{PT_LOAD, SYS_PROCESS_PARAM_MAGIC};
    let phoff: usize = 64;
    let phentsize: usize = 56;
    let pt_load_offset: usize = 0x100;
    let pt_load_size: usize = 0x40;
    let payload_offset: usize = pt_load_offset; // struct starts here
    let total = payload_offset + pt_load_size + 32;
    let mut data = vec![0u8; total];
    data[0..4].copy_from_slice(b"\x7fELF");
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
    data[54..56].copy_from_slice(&(phentsize as u16).to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());
    data[phoff..phoff + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[phoff + 8..phoff + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
    data[phoff + 16..phoff + 24].copy_from_slice(&p_vaddr.to_be_bytes());
    data[phoff + 32..phoff + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    data[phoff + 40..phoff + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
    let start = payload_offset;
    data[start..start + 4].copy_from_slice(&struct_size.to_be_bytes());
    data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
    data
}

#[test]
fn build_classifier_context_overflows_on_sys_proc_param_addr_near_u64_max() {
    // Positive control so the overflow assertion below is not vacuous.
    let normal_eboot = synthetic_eboot_with_sys_proc_param_at(0x10_0000, 0x30);
    let normal_obs = obs(ObservedOutcome::Completed, vec![]);
    let normal_ctx = build_classifier_context(&normal_eboot, &normal_obs).unwrap();
    assert!(normal_ctx.sys_proc_param_range.is_some());

    let observation = obs(ObservedOutcome::Completed, vec![]);
    let eboot = synthetic_eboot_with_sys_proc_param_at(u64::MAX - 0x10, 0x30);
    assert!(matches!(
        build_classifier_context(&eboot, &observation),
        Err(FixtureGenError::SysProcParamAddrOverflow { .. })
    ));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "IdentityMismatch invariant violated")]
fn classify_all_panics_on_addr_mismatch_in_debug() {
    use cellgov_compare::{
        ByteDivergence, EventCompare, RegionCompareSummary, StateHashCompare, StepCompare,
    };
    let cellgov = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    let result = ObservationCompareResult {
        outcome_match: true,
        a_outcome: ObservedOutcome::Completed,
        b_outcome: ObservedOutcome::Completed,
        region_compare: RegionCompareSummary {
            a_count: 1,
            b_count: 1,
            pairs: vec![RegionPairOutcome::ByteDivergence {
                name: "code".to_string(),
                addr: 0x20000, // != cellgov's 0x10000
                length: 4,
                bytes: vec![ByteDivergence {
                    offset: 0,
                    length: 1,
                    a_byte: 0,
                    b_byte: 0xFF,
                }],
            }],
        },
        event_compare: EventCompare::Equal { count: 0 },
        state_hash_compare: StateHashCompare::NoHashInfo,
        step_compare: StepCompare::NoStepInfo,
        a_runner: "cellgov".to_string(),
        b_runner: "rpcs3".to_string(),
    };
    let _ = classify_all(&result, &cellgov, &ClassifierContext::default());
}

#[test]
fn classify_all_returns_one_class_per_byte_divergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x17] = 0xAA;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
    );
    let result = compare_observations(&a, &b);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    let classes = classify_all(&result, &a, &ctx);
    assert_eq!(classes, vec![DivergenceClass::ElfHeader]);
}

#[test]
fn classify_all_returns_unclassified_without_dying() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0u8; 8])],
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0xFFu8; 8])],
    );
    let result = compare_observations(&a, &b);
    let classes = classify_all(&result, &a, &ClassifierContext::default());
    assert_eq!(classes, vec![DivergenceClass::Unclassified]);
}

#[test]
fn render_summary_section_non_semantic_lists_per_class_and_lowest_offset() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x17] = 0xAA;
    b_data[0x35] = 0xBB;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
    );
    let result = compare_observations(&a, &b);
    let summary = summarize(
        &result,
        &[DivergenceClass::ElfHeader, DivergenceClass::ElfHeader],
    );
    let section = render_summary_section(&summary, &a, &b);
    assert!(section.contains("Total non-semantic bytes: 2"));
    assert!(section.contains("ElfHeader: 2 bytes"));
    assert!(section.contains("Lowest-offset divergence: ElfHeader"));
    assert!(section.contains("code@0x10000"));
}

#[test]
fn render_summary_section_pending_enumerates_runs_with_bytes() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, vec![0u8; 0x40]),
            region("data", 0x80000, vec![0x00u8; 4]),
        ],
    );
    let mut b_code = vec![0u8; 0x40];
    b_code[0x17] = 0xAA;
    let b_data = vec![0xAA, 0xBB, 0xCC, 0xDD];
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, b_data),
        ],
    );
    let result = compare_observations(&a, &b);
    let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::Unclassified];
    let summary = summarize(&result, &classes);
    let section = render_summary_section(&summary, &a, &b);
    assert!(
        section.contains("Pending bytes: 4 across 1 run(s)"),
        "section missing pending header: {section}"
    );
    assert!(
        section.contains("data@0x0+4"),
        "section missing per-run locator: {section}"
    );
    assert!(
        section.contains("cellgov=00000000") && section.contains("rpcs3=aabbccdd"),
        "section missing inline bytes: {section}"
    );
}

#[test]
fn render_summary_section_diverge_explains_undefined_byte_parity() {
    let a = obs(ObservedOutcome::Fault, vec![region("r", 0, vec![0u8; 4])]);
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0u8; 4])],
    );
    let result = compare_observations(&a, &b);
    let summary = summarize(&result, &[]);
    let section = render_summary_section(&summary, &a, &b);
    assert!(section.contains("Byte parity is undefined"));
    assert!(section.contains("did not converge"));
    assert!(section.contains("outcome: Fault vs Completed"));
}

#[test]
fn render_summary_section_multiple_classes_are_byte_deterministic() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, vec![0u8; 0x40]),
            region("data", 0x80000, vec![0x00u8; 8]),
        ],
    );
    let mut b_code = vec![0u8; 0x40];
    b_code[0x17] = 0xAA;
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, vec![0xFFu8; 8]),
        ],
    );
    let result = compare_observations(&a, &b);
    let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::Unclassified];
    let summary = summarize(&result, &classes);
    let first = render_summary_section(&summary, &a, &b);
    let second = render_summary_section(&summary, &a, &b);
    assert_eq!(first, second);
    assert!(first.contains("ElfHeader: 1 bytes"));
    assert!(first.contains("Unclassified: 8 bytes"));
}

#[test]
fn render_unclassified_run_summarises_long_runs_with_head_tail() {
    let mut a_data = vec![0u8; 100];
    for (i, b) in a_data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let mut b_data = vec![0u8; 100];
    for (i, b) in b_data.iter_mut().enumerate() {
        *b = (i + 0x80) as u8;
    }
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, a_data)],
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, b_data)],
    );
    let run = UnclassifiedRun {
        region_name: "data".to_string(),
        offset: 0,
        length: 100,
    };
    let line = render_unclassified_run(&run, &a, &b);
    assert!(line.contains("data@0x0+100"));
    assert!(line.contains(".."));
    assert!(line.contains("(100 bytes)"));
    assert!(
        line.contains("cellgov=0001020304050607") && line.contains("rpcs3=8081828384858687"),
        "head bytes missing: {line}"
    );
}

#[test]
fn render_unclassified_run_names_runner_missing_region() {
    let with_region = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0u8; 4])],
    );
    let without_region = obs(ObservedOutcome::Completed, vec![]);
    let run = UnclassifiedRun {
        region_name: "data".to_string(),
        offset: 0,
        length: 4,
    };
    let only_cellgov = render_unclassified_run(&run, &with_region, &without_region);
    assert!(
        only_cellgov.contains("(region missing in rpcs3 observation)"),
        "got: {only_cellgov}"
    );
    let only_rpcs3 = render_unclassified_run(&run, &without_region, &with_region);
    assert!(
        only_rpcs3.contains("(region missing in cellgov observation)"),
        "got: {only_rpcs3}"
    );
    let neither = render_unclassified_run(&run, &without_region, &without_region);
    assert!(
        neither.contains("(region missing in both observations)"),
        "got: {neither}"
    );
}

#[test]
fn region_slice_empty_length_returns_empty_vec() {
    let o = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0xAA; 4])],
    );
    assert_eq!(region_slice(&o, "r", 0, 0), Some(Vec::new()));
}

#[test]
fn region_slice_offset_at_end_with_zero_length_is_some_empty() {
    let o = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0xAA; 4])],
    );
    assert_eq!(region_slice(&o, "r", 4, 0), Some(Vec::new()));
}

#[test]
fn region_slice_offset_plus_length_at_end_is_inclusive_some() {
    let o = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0xAA, 0xBB, 0xCC, 0xDD])],
    );
    assert_eq!(region_slice(&o, "r", 2, 2), Some(vec![0xCC, 0xDD]));
}

#[test]
fn region_slice_offset_plus_length_past_end_is_none() {
    let o = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0xAA; 4])],
    );
    assert_eq!(region_slice(&o, "r", 3, 2), None);
}

#[test]
fn fixture_gen_produces_byte_deterministic_output_across_two_invocations() {
    let tmp = std::env::temp_dir().join(format!("cellgov_fixture_gen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    struct CleanUp(std::path::PathBuf);
    impl Drop for CleanUp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = CleanUp(tmp.clone());

    let a_obs = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 0x40])],
    );
    let mut b_data = vec![0u8; 0x40];
    b_data[0x17] = 0xAA;
    let b_obs = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
    );
    let result = compare_observations(&a_obs, &b_obs);
    let ctx = ClassifierContext {
        elf_header_range: Some(0x10000..0x10040),
        ..ClassifierContext::default()
    };
    let classes = classify_all(&result, &a_obs, &ctx);
    let summary = summarize(&result, &classes);

    write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
    let first = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
    write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
    let second = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
    assert_eq!(first, second, "two renders must produce identical bytes");
    assert!(first.contains("Convergence: Yes"));
    assert!(first.contains("Byte parity: 1 non-semantic"));
}
