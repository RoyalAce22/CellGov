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
#[path = "tests/prescan_format_tests.rs"]
mod tests;
