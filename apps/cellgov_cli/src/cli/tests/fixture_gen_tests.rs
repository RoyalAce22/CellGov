//! Fixture-gen template substitution and report rendering.

use super::*;
use cellgov_compare::{
    compare_observations, ClassifierContext, DivergenceClass, NamedMemoryRegion,
    ObservationMetadata, ObservedOutcome,
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
    let tmp = cellgov_testkit::scratch::scratch_labeled("fixture_gen");

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
    let classes = classify_all(&result, &a_obs, &b_obs, &ctx);
    let summary = summarize(&result, &classes);

    write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
    let first = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
    write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
    let second = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
    assert_eq!(first, second, "two renders must produce identical bytes");
    assert!(first.contains("Convergence: Yes"));
    assert!(first.contains("Byte parity: 1 non-semantic"));
}

/// The oracle-gap count against a scratch VFS root and fixture tree.
mod oracle_gap_count_reads {
    use super::*;

    const CONTENT_ID: &str = "TEST00000";

    fn cell() -> CellKey {
        CellKey {
            fw: "4.93".to_string(),
            game_ver: None,
        }
    }

    fn write_overlay(root: &Path, text: &str) {
        std::fs::create_dir_all(root.join(".cellgov")).unwrap();
        std::fs::write(root.join(".cellgov/oracle-gap.tsv"), text).unwrap();
    }

    fn write_anchor(fixtures: &Path, text: &str) {
        let path = crate::paths::boot_anchor_path_in(fixtures, CONTENT_ID, &cell());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const OVERLAY: &str = "revision\tabc123\nordinal\n14\n";

    #[test]
    fn an_absent_overlay_is_no_count_even_beside_a_malformed_anchor() {
        let tmp = cellgov_testkit::scratch::scratch_labeled("oracle_gap_absent");
        write_anchor(&tmp.join("fixtures"), "{");
        let got = oracle_gap_count(
            &tmp.join("root"),
            &tmp.join("fixtures"),
            CONTENT_ID,
            &cell(),
        );
        assert_eq!(got, Ok(None));
    }

    #[test]
    fn an_absent_anchor_is_no_count() {
        let tmp = cellgov_testkit::scratch::scratch_labeled("oracle_gap_no_anchor");
        write_overlay(&tmp.join("root"), OVERLAY);
        let got = oracle_gap_count(
            &tmp.join("root"),
            &tmp.join("fixtures"),
            CONTENT_ID,
            &cell(),
        );
        assert_eq!(got, Ok(None));
    }

    #[test]
    fn an_overlay_path_that_cannot_be_read_is_refused() {
        let tmp = cellgov_testkit::scratch::scratch_labeled("oracle_gap_unreadable");
        std::fs::create_dir_all(tmp.join("root/.cellgov/oracle-gap.tsv")).unwrap();
        let got = oracle_gap_count(
            &tmp.join("root"),
            &tmp.join("fixtures"),
            CONTENT_ID,
            &cell(),
        );
        assert!(
            got.as_ref()
                .is_err_and(|e| e.starts_with("read oracle-gap overlay")),
            "got: {got:?}"
        );
    }

    #[test]
    fn a_malformed_overlay_is_refused_by_line() {
        let tmp = cellgov_testkit::scratch::scratch_labeled("oracle_gap_malformed");
        write_overlay(&tmp.join("root"), "revision\tabc123\nordinal\nx\n");
        let got = oracle_gap_count(
            &tmp.join("root"),
            &tmp.join("fixtures"),
            CONTENT_ID,
            &cell(),
        );
        assert!(
            got.as_ref().is_err_and(|e| e.contains("line 3")),
            "got: {got:?}"
        );
    }

    #[test]
    fn a_present_anchor_that_is_not_a_boot_summary_is_refused() {
        let tmp = cellgov_testkit::scratch::scratch_labeled("oracle_gap_bad_anchor");
        write_overlay(&tmp.join("root"), OVERLAY);
        write_anchor(&tmp.join("fixtures"), "{");
        let got = oracle_gap_count(
            &tmp.join("root"),
            &tmp.join("fixtures"),
            CONTENT_ID,
            &cell(),
        );
        assert!(
            got.as_ref()
                .is_err_and(|e| e.starts_with("parse boot anchor")),
            "got: {got:?}"
        );
    }
}
