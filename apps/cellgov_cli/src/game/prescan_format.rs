//! Prescan render-side formatting: collapse reserved-primary
//! [`EncodingNotRecognized`] rows into one synthetic `data-in-text`
//! line when [`CoverageMode`] cannot prove which walked bytes were
//! instructions.
//!
//! [`EncodingNotRecognized`]: cellgov_ppu::instruction::PpuDecodeError::EncodingNotRecognized

use cellgov_ppu::instruction::PpuDecodeError;
use cellgov_ppu::prescan::{CoverageMode, ElfTextCoverage, PrescanGap, PrescanReport};

/// True for raw words whose primary opcode is reserved/illegal in
/// the PowerPC PPU ISA. Primary 0 only.
pub fn is_reserved_primary(raw: u32) -> bool {
    (raw >> 26) == 0
}

/// Render the prescan report as a sequence of stderr-bound lines.
///
/// In [`CoverageMode::SectionFilteredAnonymous`] and
/// [`CoverageMode::SegmentFallback`], reserved-primary
/// `EncodingNotRecognized` gaps fold into a single synthetic
/// `data-in-text` line; all other gaps print verbatim. Other
/// coverage modes do not bucket.
pub fn format_prescan_report(
    report: &PrescanReport,
    coverage: &ElfTextCoverage,
    elf_path: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mode_note = match coverage.mode {
        CoverageMode::SectionFilteredAnonymous => {
            " (section names absent; precision is segment-grade)"
        }
        _ => "",
    };
    out.push(format!(
        "prescan: walked {} segments, {} bytes, {} instructions \
         ({} accepted, {} stubbed, {} rejected) in {}; coverage \
         mode={:?}{}",
        coverage.executable_segments,
        coverage.bytes_scanned,
        report.words_scanned,
        report.words_accepted,
        report.words_stubbed,
        report.words_rejected,
        elf_path,
        coverage.mode,
        mode_note,
    ));

    if !report.has_gaps() {
        out.push("prescan: no rejected encodings in walked text".to_string());
        return out;
    }

    let bucket_eligible = matches!(
        coverage.mode,
        CoverageMode::SectionFilteredAnonymous | CoverageMode::SegmentFallback
    );

    let mut bucket_distinct: u64 = 0;
    let mut bucket_words: u64 = 0;
    let mut verbatim: Vec<&PrescanGap> = Vec::new();
    for gap in &report.gaps {
        let bucketable = bucket_eligible
            && matches!(gap.error,
                PpuDecodeError::EncodingNotRecognized { raw } if is_reserved_primary(raw));
        if bucketable {
            bucket_distinct += 1;
            bucket_words += gap.occurrences;
        } else {
            verbatim.push(gap);
        }
    }
    let verbatim_words: u64 = verbatim.iter().map(|g| g.occurrences).sum();
    debug_assert_eq!(
        bucket_words + verbatim_words,
        report.words_rejected,
        "prescan formatter conservation broken: bucket {bucket_words} + verbatim {verbatim_words} != rejected {}",
        report.words_rejected,
    );

    let printed_distinct = verbatim.len() + if bucket_distinct > 0 { 1 } else { 0 };
    out.push(format!(
        "prescan: {printed_distinct} distinct rejected encoding(s):"
    ));
    if bucket_distinct > 0 {
        out.push(format!(
            "  data-in-text (reserved primary 0, {:?}): {bucket_distinct} distinct encodings, {bucket_words} words",
            coverage.mode,
        ));
    }
    for gap in verbatim {
        out.push(format!("  {}x  {}", gap.occurrences, gap.error));
    }
    out.push(
        "prescan: runtime DecoderArmUnimplemented / EncodingNotRecognized is \
         the co-equal backstop for code outside this static reach"
            .to_string(),
    );
    out
}

#[cfg(test)]
mod tests {
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
}
