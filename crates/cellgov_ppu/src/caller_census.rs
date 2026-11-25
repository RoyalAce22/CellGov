//! Finds LV2 syscall and LV1 hypercall sites in a PPU ELF, and groups
//! one module's syscall sites the way the caller tables record them.

use std::collections::{BTreeMap, BTreeSet};

use crate::funcmap::{self, FuncMapError, FunctionName};
use crate::instruction::{branch_target, PpuInstruction};
use crate::loader::{pt_load_segments, LoadError, LoadSegment};
use cellgov_ps3_abi::format::elf::{ELF_E_MACHINE_OFFSET, ELF_HEADER_SIZE, EM_PPC64};
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
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

/// Find every LV1 hypercall site, an `sc` with LEV = 1, in executable,
/// file-backed segments. A site's `ordinal` is the hypercall number.
///
/// Resolution is the one [`scan_syscalls`] uses, with three differences:
/// - any non-negative constant resolves;
/// - a conditional branch without link keeps the constant, because the
///   path that falls through it to `sc` does not write r11. The kernel's
///   wrappers test a flag between `li r11` and `sc`;
/// - a direct branch of the segment from outside the window that
///   enters it after `li r11` leaves the site unresolved: that path
///   reaches `sc` with another r11. The window runs from `li r11` to
///   `sc`.
///
/// The segments may lie anywhere in the 64-bit address space, as a
/// kernel's do. The scan does not see an indirect branch or a branch
/// from another segment.
///
/// # Errors
///
/// Returns [`CallerScanError`] if the ELF or an executable segment is
/// malformed.
pub fn scan_hypercalls(elf: &[u8]) -> Result<Vec<CallerSite>, CallerScanError> {
    scan_elf(elf, validate_segment_extent, |base, words| {
        scan_sc_sites(
            base,
            words,
            ScanRule {
                lev: 1,
                limit: u64::from(u16::MAX) + 1,
                through_conditional: true,
                entries: true,
            },
        )
    })
}

/// Which `sc` sites a scan finds and how it resolves them.
#[derive(Debug, Clone, Copy)]
struct ScanRule {
    lev: u8,
    /// A site resolves to an r11 constant below this.
    limit: u64,
    /// Whether the constant survives a conditional branch without link.
    through_conditional: bool,
    /// Whether a direct branch into the window leaves the site unresolved.
    entries: bool,
}

/// Each direct branch target in `words`, which start at `base`, with
/// the addresses of the branches that reach it.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + the displacement, or the displacement itself when AA = 1.
fn direct_branch_entries(base: u64, words: &[u32]) -> BTreeMap<u64, Vec<u64>> {
    let mut entries: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for (index, raw) in words.iter().copied().enumerate() {
        let source = base + (index as u64) * 4;
        let target = match crate::decode::decode(raw) {
            Ok(PpuInstruction::B { offset, aa, .. }) => branch_target(source, offset, aa),
            Ok(PpuInstruction::Bc { offset, aa, .. }) => {
                branch_target(source, i32::from(offset), aa)
            }
            _ => continue,
        };
        entries.entry(target).or_default().push(source);
    }
    entries
}

/// Whether a branch from outside `[load, site]` reaches an address in
/// `(load, site]`, so a path skips the load of r11 at `load`.
fn entered_after_load(entries: &BTreeMap<u64, Vec<u64>>, load: u64, site: u64) -> bool {
    entries
        .range(load + 4..=site)
        .flat_map(|(_, sources)| sources)
        .any(|&source| source < load || source > site)
}

/// Whether `decoded` is a branch that can fall through without writing
/// LR: a conditional `bc` that does not link.
///
/// [PPC-Book1 p:20 s:2.4.1] `BO` encoding `1z1zz`: branch always.
fn falls_through(decoded: Option<&PpuInstruction>) -> bool {
    matches!(decoded, Some(PpuInstruction::Bc { bo, link: false, .. }) if bo & PPC_BO_ALWAYS != PPC_BO_ALWAYS)
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
    for segment in segments.into_iter().filter(|segment| segment.executable) {
        validate(&segment)?;
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
        sites.extend(scan(segment.vaddr, &words));
    }
    sites.sort_by_key(|site| site.address);
    Ok(sites)
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

fn scan_words(base: u64, words: &[u32]) -> Vec<CallerSite> {
    scan_sc_sites(
        base,
        words,
        ScanRule {
            lev: 0,
            limit: SYSCALL_TABLE_SLOTS,
            through_conditional: false,
            entries: false,
        },
    )
}

/// The `sc` sites `rule` names in `words`, which start at `base`.
fn scan_sc_sites(base: u64, words: &[u32], rule: ScanRule) -> Vec<CallerSite> {
    let ScanRule {
        lev,
        limit,
        through_conditional,
        entries,
    } = rule;
    let entries = if entries {
        direct_branch_entries(base, words)
    } else {
        BTreeMap::new()
    };
    let mut sites = Vec::new();
    // The constant, and the count of words between its load and here.
    let mut candidate: Option<(u16, u8)> = None;
    for (index, raw) in words.iter().copied().enumerate() {
        let address = base + (index as u64) * 4;
        let decoded = crate::decode::decode(raw);
        if let Ok(PpuInstruction::Sc { lev: site_lev }) = decoded {
            if site_lev == lev {
                let ordinal = candidate
                    .filter(|&(_, age)| {
                        let load = address - 4 * (u64::from(age) + 1);
                        !entered_after_load(&entries, load, address)
                    })
                    .map(|(ordinal, _)| ordinal);
                sites.push(CallerSite { address, ordinal });
            }
            candidate = None;
            continue;
        }
        if let Ok(PpuInstruction::Addi { rt: 11, ra: 0, imm }) = decoded {
            candidate = u16::try_from(imm)
                .ok()
                .filter(|ordinal| u64::from(*ordinal) < limit)
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
        if !(through_conditional && falls_through(decoded.as_ref().ok()))
            && matches!(
                decoded,
                Ok(PpuInstruction::B { .. }
                    | PpuInstruction::Bc { .. }
                    | PpuInstruction::Bclr { .. }
                    | PpuInstruction::Bcctr { .. })
            )
        {
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

fn maybe_clobbers_r11(raw: u32) -> bool {
    let primary = raw >> 26;
    let rt = (raw >> 21) & 0x1f;
    let ra = (raw >> 16) & 0x1f;
    match primary {
        7 | 8 | 12..=15 | 32 | 34 | 36 | 38 | 40 | 42 | 46 => rt == 11,
        20..=30 => ra == 11,
        31 => rt == 11 || ra == 11,
        33 | 35 | 37 | 39 | 41 | 43 | 45 => rt == 11 || ra == 11,
        // ldu (DS XO 1) also writes the EA into RA [PowerISA-3.1 p:I57].
        58 => rt == 11 || (raw & 3 == 1 && ra == 11),
        // lfsu, lfdu, stfsu and stfdu write the EA into RA and target an
        // FPR [PowerISA-3.1 p:I150] [PowerISA-3.1 p:I152]
        // [PowerISA-3.1 p:I155] [PowerISA-3.1 p:I157].
        49 | 51 | 53 | 55 => ra == 11,
        62 => ra == 11,
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
