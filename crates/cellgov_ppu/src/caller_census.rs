//! Finds LV2 syscall sites in a PPU ELF.

use crate::instruction::PpuInstruction;
use crate::loader::{pt_load_segments, LoadError, LoadSegment};
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

/// Maximum instructions between a constant load into `r11` and `sc`.
pub const SYSCALL_LOOKBACK: u8 = 12;

/// One LV2 syscall site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerSite {
    /// Guest address of `sc`.
    pub address: u64,
    /// Constant `r11` value, or `None` when the bounded scan cannot prove one.
    pub ordinal: Option<u16>,
}

/// Errors from scanning a PPU ELF for syscall sites.
#[derive(Debug, thiserror::Error)]
pub enum CallerScanError {
    /// The ELF load segments are malformed.
    #[error("caller census: {0}")]
    Elf(#[from] LoadError),
    /// A file-backed executable segment leaves the input bytes.
    #[error("caller census: executable segment {index} is truncated")]
    SegmentTruncated {
        /// Program-header index of the truncated segment.
        index: usize,
    },
    /// An executable segment declares more file bytes than memory bytes.
    #[error(
        "caller census: executable segment {index} filesz 0x{filesz:x} exceeds memsz 0x{memsz:x}"
    )]
    SegmentFileszExceedsMemsz {
        /// Program-header index of the malformed segment.
        index: usize,
        /// File-backed byte count.
        filesz: u64,
        /// In-memory byte count.
        memsz: u64,
    },
    /// An executable segment does not fit the 32-bit PS3 address space.
    #[error(
        "caller census: executable segment {index} address range is outside the PPU address space"
    )]
    SegmentOutOfRange {
        /// Program-header index of the malformed segment.
        index: usize,
    },
}

/// Find every LV2 `sc` site in executable, file-backed segments.
///
/// Resolution is conservative. A site resolves only when a straight-line
/// sequence loads a non-negative table ordinal into r11 and no later
/// instruction may replace it.
///
/// # Errors
///
/// Returns [`CallerScanError`] if the ELF or an executable segment is malformed.
pub fn scan_syscalls(elf: &[u8]) -> Result<Vec<CallerSite>, CallerScanError> {
    let segments = pt_load_segments(elf)?;
    let mut sites = Vec::new();
    for segment in segments.into_iter().filter(|segment| segment.executable) {
        validate_segment(&segment)?;
        let start = usize::try_from(segment.file_offset).map_err(|_| {
            CallerScanError::SegmentTruncated {
                index: segment.index,
            }
        })?;
        let len =
            usize::try_from(segment.filesz).map_err(|_| CallerScanError::SegmentTruncated {
                index: segment.index,
            })?;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= elf.len())
            .ok_or(CallerScanError::SegmentTruncated {
                index: segment.index,
            })?;
        let words: Vec<u32> = elf[start..end]
            .chunks_exact(4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect();
        sites.extend(scan_words(segment.vaddr, &words));
    }
    sites.sort_by_key(|site| site.address);
    Ok(sites)
}

fn validate_segment(segment: &LoadSegment) -> Result<(), CallerScanError> {
    if segment.filesz > segment.memsz {
        return Err(CallerScanError::SegmentFileszExceedsMemsz {
            index: segment.index,
            filesz: segment.filesz,
            memsz: segment.memsz,
        });
    }
    let end =
        segment
            .vaddr
            .checked_add(segment.memsz)
            .ok_or(CallerScanError::SegmentOutOfRange {
                index: segment.index,
            })?;
    if end > u64::from(u32::MAX) + 1 {
        return Err(CallerScanError::SegmentOutOfRange {
            index: segment.index,
        });
    }
    Ok(())
}

fn scan_words(base: u64, words: &[u32]) -> Vec<CallerSite> {
    let mut sites = Vec::new();
    let mut candidate: Option<(u16, u8)> = None;
    for (index, raw) in words.iter().copied().enumerate() {
        let address = base + (index as u64) * 4;
        let decoded = crate::decode::decode(raw);
        if let Ok(PpuInstruction::Sc { lev }) = decoded {
            if lev == 0 {
                sites.push(CallerSite {
                    address,
                    ordinal: candidate.map(|(ordinal, _)| ordinal),
                });
            }
            candidate = None;
            continue;
        }
        if let Ok(PpuInstruction::Addi { rt: 11, ra: 0, imm }) = decoded {
            candidate = u16::try_from(imm)
                .ok()
                .filter(|ordinal| u64::from(*ordinal) < SYSCALL_TABLE_SLOTS)
                .map(|ordinal| (ordinal, 0));
            continue;
        }
        if matches!(
            decoded,
            Ok(PpuInstruction::Addi { rt: 11, .. }
                | PpuInstruction::Addis { rt: 11, .. }
                | PpuInstruction::Or { ra: 11, .. })
        ) {
            candidate = None;
            continue;
        }
        if matches!(
            decoded,
            Ok(PpuInstruction::B { .. }
                | PpuInstruction::Bc { .. }
                | PpuInstruction::Bclr { .. }
                | PpuInstruction::Bcctr { .. })
        ) {
            candidate = None;
            continue;
        }
        if candidate.is_some()
            && (maybe_clobbers_r11(raw)
                || matches!(
                    decoded,
                    Ok(PpuInstruction::Lmw { .. }
                        | PpuInstruction::Lswi { .. }
                        | PpuInstruction::Lswx { .. })
                )
                || decoded.is_err())
        {
            candidate = None;
            continue;
        }
        if let Some((ordinal, age)) = candidate {
            candidate = (age < SYSCALL_LOOKBACK).then_some((ordinal, age + 1));
        }
    }
    sites
}

fn maybe_clobbers_r11(raw: u32) -> bool {
    let primary = raw >> 26;
    let rt = (raw >> 21) & 0x1f;
    let ra = (raw >> 16) & 0x1f;
    match primary {
        7 | 8 | 12..=15 | 32 | 34 | 36 | 38 | 40 | 42 | 46 | 58 => rt == 11,
        20..=30 => ra == 11,
        31 => rt == 11 || ra == 11,
        33 | 35 | 37 | 39 | 41 | 43 | 45 => rt == 11 || ra == 11,
        62 => ra == 11,
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests/caller_census_tests.rs"]
mod tests;
