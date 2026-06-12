//! Prescan-report rendering -- decode-gap bucketing and coverage lines.

use super::*;
use cellgov_ppu::instruction::Locator;

/// (a) several primary-0 EncodingNotRecognized, (b) one SPR
/// DecoderArmUnimplemented, (c) one EncodingNotRecognized whose
/// primary is documented (primary 14, addi-class -- the
/// decoder doesn't actually emit this since primary 14 is
/// accepted, but the bucketer logic is purely render-side, so
/// constructing the synthetic shape is fine).
fn mixed_report() -> PrescanReport {
    PrescanReport {
        words_scanned: 1000,
        words_accepted: 700,
        words_stubbed: 50,
        words_rejected: 1247 + 664 + 134 + 5 + 7,
        gaps: vec![
            PrescanGap {
                error: PpuDecodeError::EncodingNotRecognized { raw: 0x0000_0000 },
                occurrences: 1247,
            },
            PrescanGap {
                error: PpuDecodeError::EncodingNotRecognized { raw: 0x0000_0001 },
                occurrences: 664,
            },
            PrescanGap {
                error: PpuDecodeError::EncodingNotRecognized { raw: 0x0000_0150 },
                occurrences: 134,
            },
            PrescanGap {
                error: PpuDecodeError::DecoderArmUnimplemented {
                    locator: Locator::Spr {
                        op_mnemonic: "mfspr",
                        spr: 18,
                    },
                    mnemonic: "mfdsisr",
                    raw: (31u32 << 26) | (3u32 << 21) | (18u32 << 16) | (339u32 << 1),
                },
                occurrences: 5,
            },
            PrescanGap {
                error: PpuDecodeError::EncodingNotRecognized { raw: 0x3800_0000 },
                occurrences: 7,
            },
        ],
    }
}

fn coverage_with_mode(mode: CoverageMode) -> ElfTextCoverage {
    ElfTextCoverage {
        executable_segments: 1,
        sections_scanned: 4,
        bytes_scanned: 1_000_000,
        mode,
    }
}

#[test]
fn reserved_primary_predicate_admits_only_primary_zero() {
    assert!(is_reserved_primary(0x0000_0000));
    assert!(is_reserved_primary(0x0000_0001));
    assert!(is_reserved_primary(0x03FF_FFFF));
    assert!(!is_reserved_primary(0x0400_0000)); // primary 1
    assert!(!is_reserved_primary(0x3800_0000)); // primary 14 (addi)
    assert!(!is_reserved_primary(0x7C00_0000)); // primary 31
}

#[test]
fn anonymous_mode_buckets_primary_zero_and_keeps_others_verbatim() {
    let r = mixed_report();
    let c = coverage_with_mode(CoverageMode::SectionFilteredAnonymous);
    let lines = format_prescan_report(&r, &c, "<synth>");
    let bucket_line = lines
        .iter()
        .find(|l| l.contains("data-in-text"))
        .expect("bucket fires");
    assert!(bucket_line.contains("3 distinct encodings"));
    assert!(bucket_line.contains(&format!("{} words", 1247 + 664 + 134)));
    assert!(bucket_line.contains("SectionFilteredAnonymous"));
    let verbatim: Vec<&String> = lines.iter().filter(|l| l.starts_with("  ")).collect();
    assert_eq!(verbatim.len(), 3, "1 bucket + 2 verbatim rows");
    assert!(lines.iter().any(|l| l.contains("missing mfdsisr")));
    assert!(lines
        .iter()
        .any(|l| l.contains("no documented encoding for raw 0x38000000")));
    let header = lines
        .iter()
        .find(|l| l.contains("distinct rejected encoding"))
        .expect("header");
    assert!(header.contains("3 distinct"));
}

#[test]
fn segment_fallback_mode_buckets_primary_zero_too() {
    let r = mixed_report();
    let c = coverage_with_mode(CoverageMode::SegmentFallback);
    let lines = format_prescan_report(&r, &c, "<synth>");
    assert!(lines.iter().any(|l| l.contains("data-in-text")));
    assert!(lines.iter().any(|l| l.contains("missing mfdsisr")));
    assert!(lines
        .iter()
        .any(|l| l.contains("no documented encoding for raw 0x38000000")));
}

#[test]
fn section_filtered_named_prints_every_row_verbatim() {
    // In SectionFiltered (named), every reported row is a real
    // gap by contract -- no bucketing is allowed.
    let r = mixed_report();
    let c = coverage_with_mode(CoverageMode::SectionFiltered);
    let lines = format_prescan_report(&r, &c, "<synth>");
    assert!(!lines.iter().any(|l| l.contains("data-in-text")));
    let row_count = lines.iter().filter(|l| l.starts_with("  ")).count();
    assert_eq!(row_count, r.gaps.len(), "every gap prints verbatim");
}

#[test]
fn not_run_prints_no_gaps_message_with_empty_report() {
    let r = PrescanReport::default();
    let c = coverage_with_mode(CoverageMode::NotRun);
    let lines = format_prescan_report(&r, &c, "<synth>");
    assert!(lines.iter().any(|l| l.contains("no rejected encodings")));
}

#[test]
fn conservation_identity_holds_in_every_mode() {
    // Brief: bucketed_occurrences + verbatim_occurrences ==
    // report.words_rejected, in every mode, every time.
    for mode in [
        CoverageMode::SectionFilteredAnonymous,
        CoverageMode::SegmentFallback,
        CoverageMode::SectionFiltered,
        CoverageMode::NotRun,
    ] {
        let r = mixed_report();
        let c = coverage_with_mode(mode);
        // The debug_assert inside format_prescan_report is the
        // primary guard; the test verifies it doesn't fire on
        // any mode. (Distinct-row maths is harder to assert
        // mode-blind, so we just confirm the function runs
        // clean.)
        let _ = format_prescan_report(&r, &c, "<synth>");
    }
}

#[test]
fn mode_note_appended_only_for_anonymous() {
    let r = mixed_report();
    for (mode, expect_note) in [
        (CoverageMode::SectionFilteredAnonymous, true),
        (CoverageMode::SectionFiltered, false),
        (CoverageMode::SegmentFallback, false),
        (CoverageMode::NotRun, false),
    ] {
        let c = coverage_with_mode(mode);
        let lines = format_prescan_report(&r, &c, "<synth>");
        let summary = &lines[0];
        assert!(summary.contains(&format!("{mode:?}")));
        let has_note = summary.contains("section names absent");
        assert_eq!(has_note, expect_note, "mode={mode:?}");
    }
}

#[test]
fn header_row_count_reflects_post_bucket_total() {
    // mixed_report has 5 gaps total: 3 primary-0 + 1 SPR + 1
    // non-reserved EncodingNotRecognized. In anonymous mode the
    // 3 primary-0 rows fold to one synthetic line, so the
    // header should claim 3 distinct (1 synthetic + 2
    // verbatim), not the raw 5.
    let r = mixed_report();
    let c = coverage_with_mode(CoverageMode::SectionFilteredAnonymous);
    let lines = format_prescan_report(&r, &c, "<synth>");
    let header = lines
        .iter()
        .find(|l| l.contains("distinct rejected encoding"))
        .expect("header");
    assert!(header.starts_with("prescan: 3 distinct"));
}
