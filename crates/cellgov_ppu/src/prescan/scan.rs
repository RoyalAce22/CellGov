//! Scan accumulator: word -> decoder -> deduped gap report.

use std::collections::BTreeMap;

use crate::decode::decode;
use crate::instruction::{Locator, PpuDecodeError};

use super::error::PrescanError;
use super::sections::{executable_progbits_ranges, executable_sections_anonymous, merge_ranges};

/// One row of a prescan gap report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrescanGap {
    /// Representative error for this rejection key.
    pub error: PpuDecodeError,
    /// Number of words that produced this error.
    pub occurrences: u64,
}

/// Result of walking a slice of instruction words through the
/// decoder.
///
/// Counts satisfy `words_scanned == words_accepted + words_stubbed
/// + words_rejected`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrescanReport {
    /// Number of 32-bit words walked.
    pub words_scanned: u64,
    /// Words accepted into a fully-typed variant (excludes stubs).
    pub words_accepted: u64,
    /// Words accepted into a stub variant per
    /// [`PpuInstruction::is_stub_variant`].
    ///
    /// [`PpuInstruction::is_stub_variant`]: crate::instruction::PpuInstruction::is_stub_variant
    pub words_stubbed: u64,
    /// Words rejected (sum of all gap occurrences).
    pub words_rejected: u64,
    /// Distinct rejection encodings, in `GapKey` order: opcode
    /// gaps, then SPR-direction-tagged gaps, then unrecognized raw
    /// encodings, ascending within each bucket.
    pub gaps: Vec<PrescanGap>,
}

impl PrescanReport {
    /// Whether any rejected encodings were recorded.
    pub fn has_gaps(&self) -> bool {
        !self.gaps.is_empty()
    }

    /// Number of distinct rejection encodings.
    pub fn distinct_gap_count(&self) -> usize {
        self.gaps.len()
    }
}

/// Dedup key for the scan accumulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GapKey {
    Opcode { primary: u8, xo: u16 },
    Spr { op_mnemonic: &'static str, spr: u16 },
    Unrecognized { raw: u32 },
}

fn gap_key(err: &PpuDecodeError) -> GapKey {
    match err {
        PpuDecodeError::DecoderArmUnimplemented { locator, .. } => match locator {
            Locator::Opcode { primary, xo } => GapKey::Opcode {
                primary: *primary,
                xo: *xo,
            },
            Locator::Spr { op_mnemonic, spr } => GapKey::Spr {
                op_mnemonic,
                spr: *spr,
            },
        },
        PpuDecodeError::EncodingNotRecognized { raw } => GapKey::Unrecognized { raw: *raw },
    }
}

/// Walk an iterator of 32-bit instruction words through the
/// decoder, returning the gap report.
pub fn scan_words(words: impl IntoIterator<Item = u32>) -> PrescanReport {
    let mut accum: BTreeMap<GapKey, PrescanGap> = BTreeMap::new();
    let mut report = PrescanReport::default();
    for word in words {
        report.words_scanned += 1;
        match decode(word) {
            Ok(insn) => {
                if insn.is_stub_variant() {
                    report.words_stubbed += 1;
                } else {
                    report.words_accepted += 1;
                }
            }
            Err(err) => {
                report.words_rejected += 1;
                let key = gap_key(&err);
                accum
                    .entry(key)
                    .and_modify(|gap| gap.occurrences += 1)
                    .or_insert(PrescanGap {
                        error: err,
                        occurrences: 1,
                    });
            }
        }
    }
    report.gaps = accum.into_values().collect();
    report
}

/// Walk a `&[u8]` text segment as big-endian 32-bit instruction
/// words. Trailing bytes that don't form a full word are ignored.
pub fn scan_be_bytes(bytes: &[u8]) -> PrescanReport {
    let words = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    scan_words(words)
}

/// Which path [`scan_elf_text`] took to identify the byte ranges it
/// scanned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverageMode {
    /// Scan did not run (zero PF_X segments, or `scan_elf_text`
    /// returned `Err`).
    #[default]
    NotRun,
    /// Qualifying executable section exists with a non-empty name
    /// resolved through `.shstrtab`. Gap rows are section-grade
    /// precise.
    SectionFiltered,
    /// Section table present with qualifying executable section(s)
    /// but every `sh_name` resolves to `""` (or `e_shstrndx` is
    /// [`cellgov_ps3_abi::elf::SHN_UNDEF`] / the `.shstrtab` is
    /// absent or empty). Coverage is segment-grade.
    SectionFilteredAnonymous,
    /// Section header table absent (`e_shoff == 0`); each executable
    /// PT_LOAD segment walked in full.
    SegmentFallback,
}

/// What the [`scan_elf_text`] walk covered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElfTextCoverage {
    /// Number of PT_LOAD program-header entries with PF_X set.
    pub executable_segments: u32,
    /// Number of `SHT_PROGBITS + SHF_ALLOC + SHF_EXECINSTR` sections
    /// the section walk yielded. Zero on a stripped binary.
    pub sections_scanned: u32,
    /// Total bytes walked, post-merge across (section ∩ segment)
    /// intersections; overlapping PT_LOADs counted once.
    pub bytes_scanned: u64,
    /// Which precision the scan achieved.
    pub mode: CoverageMode,
}

/// Walk every executable PT_LOAD segment in a PPU ELF and return
/// the gap report plus coverage stats.
///
/// # Errors
///
/// Returns [`PrescanError::Loader`] when the input is not a parseable
/// PPU ELF (bad magic, wrong ELF class, wrong endianness, malformed
/// program-header table). A valid ELF with zero PF_X segments returns
/// `Ok` with an empty report and zero coverage -- "scan ran, found
/// nothing to walk" -- distinct from "scan could not run."
///
/// The scan reads program-header offsets out of the loader-validated
/// `LoadSegment` list; it does NOT need guest memory or a runtime.
/// Trailing bytes that don't form a full 32-bit word are ignored per
/// [`scan_be_bytes`].
pub fn scan_elf_text(elf_data: &[u8]) -> Result<(PrescanReport, ElfTextCoverage), PrescanError> {
    let segments = crate::loader::pt_load_segments(elf_data)?;
    let sections = executable_progbits_ranges(elf_data)?;

    let mode = if sections.is_empty() {
        CoverageMode::SegmentFallback
    } else if executable_sections_anonymous(elf_data)? {
        CoverageMode::SectionFilteredAnonymous
    } else {
        CoverageMode::SectionFiltered
    };
    let mut coverage = ElfTextCoverage {
        sections_scanned: u32::try_from(sections.len()).unwrap_or(u32::MAX),
        ..ElfTextCoverage::default()
    };

    // Collect the (section ∩ segment) intersections (or whole-segment
    // ranges in fallback mode) before scanning. A single merge pass
    // dedupes overlapping PT_LOAD entries so bytes_scanned and gap
    // occurrences aren't double-counted.
    let mut intersections: Vec<(usize, usize)> = Vec::new();
    let mut had_exec_seg = false;
    for seg in segments {
        if !seg.executable {
            continue;
        }
        had_exec_seg = true;
        coverage.executable_segments = coverage.executable_segments.saturating_add(1);

        // The two `try_from` casts guard 32-bit hosts where a u64
        // segment value can exceed `usize::MAX`. A truncating
        // `as usize` would silently shrink the scanned range and
        // break determinism. Unreachable on 64-bit hosts.
        let Ok(seg_lo) = usize::try_from(seg.file_offset) else {
            continue;
        };
        let Ok(seg_sz) = usize::try_from(seg.filesz) else {
            continue;
        };
        let seg_hi = match seg_lo.checked_add(seg_sz) {
            Some(hi) => hi.min(elf_data.len()),
            None => continue,
        };
        if seg_lo >= seg_hi {
            continue;
        }

        if sections.is_empty() {
            intersections.push((seg_lo, seg_hi));
        } else {
            for &(sec_lo, sec_hi) in &sections {
                let lo = sec_lo.max(seg_lo);
                let hi = sec_hi.min(seg_hi);
                if lo < hi {
                    intersections.push((lo, hi));
                }
            }
        }
    }

    // If we never saw an executable segment, leave coverage_mode at
    // NotRun (the Default) rather than claiming a fallback walk ran.
    if had_exec_seg {
        coverage.mode = mode;
    }

    let merged = merge_ranges(intersections);

    let mut accum: BTreeMap<GapKey, PrescanGap> = BTreeMap::new();
    let mut report = PrescanReport::default();
    for (lo, hi) in merged {
        let bytes = &elf_data[lo..hi];
        coverage.bytes_scanned = coverage.bytes_scanned.saturating_add(bytes.len() as u64);
        let chunk = scan_be_bytes(bytes);
        report.words_scanned += chunk.words_scanned;
        report.words_accepted += chunk.words_accepted;
        report.words_stubbed += chunk.words_stubbed;
        report.words_rejected += chunk.words_rejected;
        for gap in chunk.gaps {
            let key = gap_key(&gap.error);
            accum
                .entry(key)
                .and_modify(|existing| existing.occurrences += gap.occurrences)
                .or_insert(gap);
        }
    }
    report.gaps = accum.into_values().collect();
    Ok((report, coverage))
}

#[cfg(test)]
#[path = "tests/scan_tests.rs"]
mod tests;
