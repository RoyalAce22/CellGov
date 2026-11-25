//! Finds LV2 syscall and LV1 hypercall sites in a PPU ELF, and groups
//! one module's syscall sites the way the caller tables record them.

use std::collections::{BTreeMap, BTreeSet};

use crate::funcmap::{self, FuncMapError, FunctionName};
use crate::hypercall_cfg::{Code, CodeSegment};
use crate::indirect_entry::{indirect_entries, FileSegment};
use crate::instruction::PpuInstruction;
use crate::loader::{pt_load_segments, LoadError, LoadSegment};
use cellgov_ps3_abi::format::elf::{ELF_E_MACHINE_OFFSET, ELF_HEADER_SIZE, EM_PPC64};
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

/// Maximum instructions between a constant load into `r11` and `sc`.
pub const SYSCALL_LOOKBACK: u8 = 12;

/// One `sc` site: an LV2 syscall or an LV1 hypercall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerSite {
    /// Guest address of `sc`.
    pub address: u64,
    /// Constant `r11` value, or `None` when the bounded scan cannot prove one.
    pub ordinal: Option<u16>,
}

/// Errors from scanning a PPU ELF for `sc` sites.
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
    /// A file-backed segment that is not executable leaves the input
    /// bytes. [`scan_hypercalls`] reads every segment for code addresses.
    #[error("caller census: data segment {index} is truncated")]
    DataSegmentTruncated {
        /// Program-header index of the truncated segment.
        index: usize,
    },
    /// A relative jump table runs past the most entries
    /// [`scan_hypercalls`] reads from one table, so the scan cannot bound
    /// every target.
    #[error("caller census: the relative jump table at 0x{start:x} runs past 0x{limit:x} entries", limit = crate::indirect_entry::JUMP_TABLE_LIMIT)]
    JumpTableTooLong {
        /// Address of the table's first entry.
        start: u64,
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
    /// An executable segment does not fit the address space the scan
    /// allows: 32 bits for [`scan_syscalls`], 64 bits for
    /// [`scan_hypercalls`].
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
    scan_elf(elf, validate_segment, scan_words)
}

/// Find every LV1 hypercall site in executable, file-backed segments.
/// A site is a word that is exactly `sc 1`
/// ([`PPC_SC_1`](cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1)), and its
/// `ordinal` is the hypercall number.
///
/// A word with a nonzero reserved field also decodes as `sc 1`, but
/// such a word is data, so it is not a site. A site resolves when every
/// path to it loads the same non-negative constant into r11 with
/// `li r11`. The paths are those of the control-flow graph of all the
/// executable segments: direct branches, which may cross segments, and
/// fall-throughs.
///
/// A site is unresolved when control can enter a word between the load
/// and `sc` in another way. These ways are the ELF entry, an interrupt
/// vector, and an indirect branch to an address that the image names.
///
/// The segments may lie anywhere in the 64-bit address space, as a
/// kernel's do.
///
/// # Errors
///
/// Returns [`CallerScanError`] if the ELF or one of its segments is
/// malformed, or if a relative jump table runs past the most entries
/// the scan reads from one table.
pub fn scan_hypercalls(elf: &[u8]) -> Result<Vec<CallerSite>, CallerScanError> {
    let segments = pt_load_segments(elf)?;
    let code = Code::new(executable_segments(
        elf,
        &segments,
        validate_segment_extent,
    )?);
    let files = file_segments(elf, &segments)?;
    let entries = indirect_entries(elf, &files, &code)?;
    let mut sites = crate::hypercall_cfg::scan(&code, &entries);
    sites.sort_by_key(|site| site.address);
    Ok(sites)
}

/// The file bytes of every segment in `segments`, in program-header
/// order.
fn file_segments<'elf>(
    elf: &'elf [u8],
    segments: &[LoadSegment],
) -> Result<Vec<FileSegment<'elf>>, CallerScanError> {
    segments
        .iter()
        .map(|segment| {
            let index = segment.index;
            let bytes = segment_bytes(elf, segment).ok_or(if segment.executable {
                CallerScanError::SegmentTruncated { index }
            } else {
                CallerScanError::DataSegmentTruncated { index }
            })?;
            Ok(FileSegment {
                vaddr: segment.vaddr,
                bytes,
            })
        })
        .collect()
}

/// The sites `scan` finds in each executable segment `validate` accepts,
/// by address.
fn scan_elf(
    elf: &[u8],
    validate: fn(&LoadSegment) -> Result<(), CallerScanError>,
    scan: impl Fn(u64, &[u32]) -> Vec<CallerSite>,
) -> Result<Vec<CallerSite>, CallerScanError> {
    let segments = pt_load_segments(elf)?;
    let mut sites = Vec::new();
    for segment in executable_segments(elf, &segments, validate)? {
        sites.extend(scan(segment.vaddr, &segment.words));
    }
    sites.sort_by_key(|site| site.address);
    Ok(sites)
}

/// The words of each executable segment in `segments`, in
/// program-header order, after `validate` accepts the segment.
fn executable_segments(
    elf: &[u8],
    segments: &[LoadSegment],
    validate: fn(&LoadSegment) -> Result<(), CallerScanError>,
) -> Result<Vec<CodeSegment>, CallerScanError> {
    let mut code = Vec::new();
    for segment in segments.iter().filter(|segment| segment.executable) {
        validate(segment)?;
        let bytes = segment_bytes(elf, segment).ok_or(CallerScanError::SegmentTruncated {
            index: segment.index,
        })?;
        code.push(CodeSegment {
            vaddr: segment.vaddr,
            words: bytes
                .chunks_exact(4)
                .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte chunk")))
                .collect(),
        });
    }
    Ok(code)
}

/// The file bytes of `segment`, or `None` when they leave `elf`.
fn segment_bytes<'elf>(elf: &'elf [u8], segment: &LoadSegment) -> Option<&'elf [u8]> {
    let start = usize::try_from(segment.file_offset).ok()?;
    let len = usize::try_from(segment.filesz).ok()?;
    elf.get(start..start.checked_add(len)?)
}

/// One PPU module's syscall sites, grouped for the caller tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleCallers {
    /// Each resolved ordinal's site addresses, ascending.
    pub by_ordinal: BTreeMap<u16, Vec<u64>>,
    /// The sites whose ordinal the scan cannot prove, ascending.
    pub unresolved: Vec<u64>,
    /// Each `(export NID, ordinal)` pair where a resolved site lies in a
    /// function the module exports by NID.
    pub reach: BTreeSet<(u32, u16)>,
}

impl ModuleCallers {
    /// How many sites resolved to an ordinal.
    #[must_use]
    pub fn resolved_sites(&self) -> usize {
        self.by_ordinal.values().map(Vec::len).sum()
    }
}

/// Why one module's callers could not be found.
#[derive(Debug, thiserror::Error)]
pub enum ModuleCallersError {
    /// The syscall scan refused the image.
    #[error(transparent)]
    Scan(#[from] CallerScanError),
    /// The function map refused the image.
    #[error(transparent)]
    FunctionMap(#[from] FuncMapError),
}

/// The syscall callers of one module image, or `None` for an image
/// that is not a PPU ELF.
///
/// # Errors
///
/// [`ModuleCallersError`] when the syscall scan or the function map
/// refuses the image.
pub fn module_callers(elf: &[u8]) -> Result<Option<ModuleCallers>, ModuleCallersError> {
    if !is_ppu_elf(elf) {
        return Ok(None);
    }
    let sites = scan_syscalls(elf)?;
    let functions = funcmap::build(elf)?;
    let mut callers = ModuleCallers::default();
    for site in sites {
        let Some(ordinal) = site.ordinal else {
            callers.unresolved.push(site.address);
            continue;
        };
        callers
            .by_ordinal
            .entry(ordinal)
            .or_default()
            .push(site.address);
        let export = u32::try_from(site.address)
            .ok()
            .and_then(|address| functions.span_at(address))
            .and_then(|span| match span.name {
                FunctionName::Nid(nid) => Some(nid),
                FunctionName::Synthetic | FunctionName::Known(_) => None,
            });
        if let Some(nid) = export {
            callers.reach.insert((nid, ordinal));
        }
    }
    Ok(Some(callers))
}

fn is_ppu_elf(elf: &[u8]) -> bool {
    elf.len() >= ELF_HEADER_SIZE
        && u16::from_be_bytes([elf[ELF_E_MACHINE_OFFSET], elf[ELF_E_MACHINE_OFFSET + 1]])
            == EM_PPC64
}

fn validate_segment(segment: &LoadSegment) -> Result<(), CallerScanError> {
    let end = validate_segment_extent_end(segment)?;
    if end > u64::from(u32::MAX) + 1 {
        return Err(CallerScanError::SegmentOutOfRange {
            index: segment.index,
        });
    }
    Ok(())
}

/// Accepts a segment whose file bytes fit its memory bytes and whose
/// end fits the 64-bit address space.
fn validate_segment_extent(segment: &LoadSegment) -> Result<(), CallerScanError> {
    validate_segment_extent_end(segment).map(|_| ())
}

/// The end address of a segment [`validate_segment_extent`] accepts.
fn validate_segment_extent_end(segment: &LoadSegment) -> Result<u64, CallerScanError> {
    if segment.filesz > segment.memsz {
        return Err(CallerScanError::SegmentFileszExceedsMemsz {
            index: segment.index,
            filesz: segment.filesz,
            memsz: segment.memsz,
        });
    }
    segment
        .vaddr
        .checked_add(segment.memsz)
        .ok_or(CallerScanError::SegmentOutOfRange {
            index: segment.index,
        })
}

/// The LV2 `sc` sites in `words`, which start at `base`.
fn scan_words(base: u64, words: &[u32]) -> Vec<CallerSite> {
    let mut sites = Vec::new();
    // The constant, and the count of words between its load and here.
    let mut candidate: Option<(u16, u8)> = None;
    for (index, raw) in words.iter().copied().enumerate() {
        let address = base + (index as u64) * 4;
        let decoded = crate::decode::decode(raw);
        if let Ok(PpuInstruction::Sc { lev }) = decoded {
            if lev == 0 {
                let ordinal = candidate.map(|(ordinal, _)| ordinal);
                sites.push(CallerSite { address, ordinal });
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
        // A refused word has no arm that names its registers, so the
        // lookback drops the candidate as it does for a word with no decode.
        if candidate.is_some()
            && (maybe_clobbers_r11(raw)
                || matches!(
                    decoded,
                    Ok(PpuInstruction::Lmw { .. }
                        | PpuInstruction::Lswi { .. }
                        | PpuInstruction::Lswx { .. }
                        | PpuInstruction::PpeIllegal { .. }
                        | PpuInstruction::PpePrivileged { .. })
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

/// Whether the word `raw` may write r11, read from its register fields
/// alone.
pub(crate) fn maybe_clobbers_r11(raw: u32) -> bool {
    maybe_writes_gpr(raw, 11)
}

/// Whether the word `raw` may write the GPR `gpr`, read from its
/// register fields alone.
pub(crate) fn maybe_writes_gpr(raw: u32, gpr: u32) -> bool {
    let primary = raw >> 26;
    let rt = (raw >> 21) & 0x1f;
    let ra = (raw >> 16) & 0x1f;
    match primary {
        7 | 8 | 12..=15 | 32 | 34 | 36 | 38 | 40 | 42 | 46 => rt == gpr,
        20..=30 => ra == gpr,
        31 => rt == gpr || ra == gpr,
        33 | 35 | 37 | 39 | 41 | 43 | 45 => rt == gpr || ra == gpr,
        // ldu (DS XO 1) also writes the EA into RA [PowerISA-3.1 p:I57].
        58 => rt == gpr || (raw & 3 == 1 && ra == gpr),
        // lfsu, lfdu, stfsu and stfdu write the EA into RA and target an
        // FPR [PowerISA-3.1 p:I150] [PowerISA-3.1 p:I152]
        // [PowerISA-3.1 p:I155] [PowerISA-3.1 p:I157].
        49 | 51 | 53 | 55 => ra == gpr,
        62 => ra == gpr,
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests/caller_census_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/caller_census_refused_tests.rs"]
mod refused_tests;

#[cfg(test)]
#[path = "tests/caller_census_update_form_tests.rs"]
mod update_form_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_tests.rs"]
mod hypercall_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_shape_tests.rs"]
mod hypercall_shape_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_entry_tests.rs"]
mod hypercall_entry_tests;
