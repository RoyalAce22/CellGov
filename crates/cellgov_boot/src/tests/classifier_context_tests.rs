//! Classifier-context construction: the header range, the HLE OPD
//! ranges, the sync-primitive scan, and one class per byte divergence.

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
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware: None,
    }
}

fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.to_string(),
        addr,
        data,
    }
}

/// A PPU ELF64 header declaring `phnum` program headers of `phentsize`
/// bytes at `phoff`, padded out to `len` bytes. Every slot is zero, so
/// none is a PT_LOAD.
fn synthetic_elf64_be_sized(phoff: u64, phentsize: u16, phnum: u16, len: usize) -> Vec<u8> {
    let mut eboot = vec![0u8; len.max(64)];
    eboot[0..4].copy_from_slice(b"\x7fELF");
    eboot[4] = 2; // ELFCLASS64
    eboot[5] = 2; // ELFDATA2MSB
    eboot[6] = 1; // EV_CURRENT
    eboot[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    eboot[32..40].copy_from_slice(&phoff.to_be_bytes());
    eboot[54..56].copy_from_slice(&phentsize.to_be_bytes());
    eboot[56..58].copy_from_slice(&phnum.to_be_bytes());
    eboot
}

/// One program header straight after the header: the table ends at
/// 0x78.
fn one_slot_elf() -> Vec<u8> {
    synthetic_elf64_be_sized(0x40, 0x38, 1, 0x78)
}

#[test]
fn build_classifier_context_populates_elf_header_when_code_region_present() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    let ctx = build_classifier_context(&one_slot_elf(), &observation).unwrap();
    assert_eq!(ctx.elf_header_range, Some(0x10000..0x10078));
}

#[test]
fn elf_header_range_widens_to_include_phdr_table() {
    // phoff=0x40 + 5 * phentsize=0x38 -> PHDR end at 0x158.
    let eboot = synthetic_elf64_be_sized(0x40, 0x38, 5, 0x158);
    assert_eq!(header_and_phdr_table_end(&eboot).unwrap(), 0x158);
}

#[test]
fn a_phdr_table_inside_the_header_ends_the_range_at_the_header() {
    // phoff=0: the one slot overlaps the header and ends at 0x38.
    let eboot = synthetic_elf64_be_sized(0, 0x38, 1, 0x40);
    assert_eq!(header_and_phdr_table_end(&eboot).unwrap(), 0x40);
}

#[test]
fn a_phdr_table_declared_past_the_eboot_end_is_refused() {
    // One byte short of the declared 0x158 table end.
    let eboot = synthetic_elf64_be_sized(0x40, 0x38, 5, 0x157);
    assert_eq!(header_and_phdr_table_end(&eboot), Err(LoadError::TooSmall));
}

#[test]
fn an_out_of_file_phdr_table_never_widens_the_elf_header_class_range() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    // Declares a 0x38000-byte PHDR table in a 64-byte image: accepted,
    // it would mark every divergent byte under 0x10000..0x48040 as
    // non-semantic ElfHeader.
    let eboot = synthetic_elf64_be_sized(0x40, 0x38, 0x1000, 64);
    assert_eq!(
        build_classifier_context(&eboot, &observation),
        Err(ClassifierContextError::ElfHeader(LoadError::TooSmall))
    );
}

#[test]
fn the_table_end_refuses_what_the_loader_refuses() {
    let mut bad_magic = one_slot_elf();
    bad_magic[0] = 0xCC;
    let mut class_32 = one_slot_elf();
    class_32[4] = 1; // ELFCLASS32
    let mut little_endian = one_slot_elf();
    little_endian[5] = 1; // ELFDATA2LSB
    let cases: [(&str, Vec<u8>, LoadError); 7] = [
        ("short", vec![0u8; 32], LoadError::TooSmall),
        ("magic", bad_magic, LoadError::BadMagic),
        ("class", class_32, LoadError::Not64Bit),
        ("byte order", little_endian, LoadError::NotBigEndian),
        (
            "offset overflow",
            synthetic_elf64_be_sized(u64::MAX, 0x38, 5, 64),
            LoadError::TooSmall,
        ),
        (
            "no table",
            synthetic_elf64_be_sized(0x40, 0x38, 0, 64),
            LoadError::NoProgramHeaders,
        ),
        (
            "slot size",
            synthetic_elf64_be_sized(0x40, 0x40, 1, 0x80),
            LoadError::BadPhentsize { phentsize: 0x40 },
        ),
    ];
    for (label, eboot, want) in cases {
        assert_eq!(header_and_phdr_table_end(&eboot), Err(want), "{label}");
    }
}

/// An initialized `sys_lwmutex_t`: free sentinel, no waiter, a valid
/// attribute, zero recursion, the kernel id, zero pad.
fn lwmutex_bytes(sleep_queue: u32) -> Vec<u8> {
    let mut b = Vec::new();
    for w in [0xffff_ffffu32, 0, 0x22, 0, sleep_queue, 0, 0, 0] {
        b.extend_from_slice(&w.to_be_bytes());
    }
    b
}

#[test]
fn build_classifier_context_scans_every_region_for_lwmutex_slots() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![
            region("data", 0x80000, lwmutex_bytes(7)),
            region("data_hi", 0x1000_0000, lwmutex_bytes(8)),
        ],
    );
    let ctx = build_classifier_context(&one_slot_elf(), &observation).unwrap();
    assert_eq!(
        ctx.sync_primitive_id_ranges,
        vec![0x80010..0x80014, 0x1000_0010..0x1000_0014]
    );
}

#[test]
fn build_classifier_context_with_no_code_region_leaves_header_none() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0u8; 4])],
    );
    let ctx = build_classifier_context(&one_slot_elf(), &observation).unwrap();
    assert!(ctx.elf_header_range.is_none());
}

#[test]
fn build_classifier_context_propagates_elf_parse_error() {
    let observation = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
    );
    let eboot = vec![0u8; 32];
    assert_eq!(
        build_classifier_context(&eboot, &observation),
        Err(ClassifierContextError::ElfHeader(LoadError::TooSmall))
    );
}

#[test]
fn hle_opd_ranges_no_imports_table_is_empty_vec() {
    assert_eq!(
        hle_opd_ranges(&one_slot_elf()).unwrap(),
        Vec::<Range<u64>>::new()
    );
}

#[test]
fn hle_opd_ranges_propagates_non_no_imports_table_errors() {
    let eboot = vec![0u8; 32];
    match hle_opd_ranges(&eboot) {
        Err(ClassifierContextError::ImportParse(e)) => assert!(
            !matches!(e, ImportParseError::NoImportsTable),
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
    let eboot = synthetic_elf64_be_sized(0x40, 0x38, 5, 0x158);
    assert!(matches!(
        build_classifier_context(&eboot, &observation),
        Err(ClassifierContextError::CodeRegionAddrOverflow { .. })
    ));
}

/// Synthetic EBOOT with a single PT_LOAD covering a
/// sys_proc_param magic struct at file offset 0x100.
fn synthetic_eboot_with_sys_proc_param_at(p_vaddr: u64, struct_size: u32) -> Vec<u8> {
    use cellgov_ps3_abi::format::elf::{PT_LOAD, SYS_PROCESS_PARAM_MAGIC};
    let phoff: usize = 64;
    let phentsize: usize = 56;
    let pt_load_offset: usize = 0x100;
    let pt_load_size: usize = 0x40;
    let payload_offset: usize = pt_load_offset; // struct starts here
    let total = payload_offset + pt_load_size + 32;
    let mut data = synthetic_elf64_be_sized(phoff as u64, phentsize as u16, 1, total);
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
        Err(ClassifierContextError::SysProcParamAddrOverflow { .. })
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
        a_identity: cellgov_compare::RunIdentity::default(),
        b_identity: cellgov_compare::RunIdentity::default(),
    };
    let _ = classify_all(
        &result,
        &cellgov,
        &cellgov.clone(),
        &ClassifierContext::default(),
    );
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
    let classes = classify_all(&result, &a, &b, &ctx);
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
    let classes = classify_all(&result, &a, &b, &ClassifierContext::default());
    assert_eq!(classes, vec![DivergenceClass::Unclassified]);
}
