//! Function-boundary detection for PS3 PPU binaries via OPD
//! descriptor walking.
//!
//! PPC64 ELFv1 reaches every function through an 8-byte OPD
//! descriptor `{ u32 code, u32 toc }`. Starts are discovered in
//! confidence order: anchor OPDs (the entry descriptor for a main
//! ELF; every export descriptor for a PRX), a bidirectional grow
//! scan around each anchor's descriptor, then a sweep of all
//! non-executable file-backed bytes validated against the anchor
//! TOC set. Prologue heuristics are not used: they miss every leaf
//! function and misfire on mid-function LR saves.
//!
//! Scope: PPU OPD discovery only. Embedded SPU ELFs (EM_SPU,
//! ELF32-BE) in non-exec segments are not mapped -- they fall
//! through the OPD sweep silently. No producer reaches SPU
//! dispatch yet, so SPU function discovery is deferred until a
//! title drives it.
//!
//! A span's `end` is the next function start, clamped to the
//! containing executable segment's file-backed range. Functions
//! with multiple returns and tail calls make `blr`-scanning a
//! heuristic; span-until-next-start is the honest contract.

use core::fmt;

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::elf::{ELF_HEADER_SIZE, ET_PRX};

use crate::loader::{pt_load_segments, LoadError, LoadSegment};
use crate::sprx::{parse_prx, PrxParseError};

/// Upper bound on scan-discovered function starts, so a pathological
/// descriptor-shaped segment cannot allocate unbounded spans. Anchor
/// inserts (entry / exports) are not counted against it; their count
/// is already bounded by the parsed export table.
const MAX_FUNCTIONS: usize = 1 << 20;

/// Function spans discovered for one binary.
#[derive(Debug, Clone)]
pub struct FunctionMap {
    /// Sorted by `start`; spans never overlap.
    pub functions: Vec<FunctionSpan>,
    /// True when discovery stopped at `MAX_FUNCTIONS`; the map is
    /// then a prefix, not the full set.
    pub truncated: bool,
}

impl FunctionMap {
    /// The span containing `addr`, if any. Binary search.
    pub fn span_at(&self, addr: u32) -> Option<&FunctionSpan> {
        let idx = self.functions.partition_point(|s| s.start <= addr);
        let span = self.functions.get(idx.checked_sub(1)?)?;
        (addr < span.end).then_some(span)
    }
}

/// One discovered function: `[start, end)` in guest vaddr space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionSpan {
    /// First instruction vaddr.
    pub start: u32,
    /// Exclusive end: next function start or end of the executable
    /// segment's file-backed range.
    pub end: u32,
    /// How the function is named; render via
    /// [`FunctionSpan::display_name`].
    pub name: FunctionName,
    /// How this start was discovered; consumers can filter.
    pub origin: FunctionOrigin,
}

impl FunctionSpan {
    /// Name for display: `sub_<start>` for synthetic spans,
    /// `nid_<NID>` for unresolved exports, the literal name
    /// otherwise.
    pub fn display_name(&self) -> impl fmt::Display + '_ {
        DisplayName(self)
    }
}

struct DisplayName<'a>(&'a FunctionSpan);

impl fmt::Display for DisplayName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.name {
            FunctionName::Known(name) => f.write_str(name),
            FunctionName::Nid(nid) => write!(f, "nid_{nid:08x}"),
            FunctionName::Synthetic => write!(f, "sub_{:08x}", self.0.start),
        }
    }
}

/// How a function is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionName {
    /// Export resolved via NID; symbol-name lookup is the caller's
    /// concern.
    Nid(u32),
    /// No name source; renders as `sub_<start>`.
    Synthetic,
    /// Entry point / `module_start` / `module_stop`.
    Known(&'static str),
}

/// Discovery source of a function start, in descending confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionOrigin {
    /// The main ELF's `e_entry` descriptor.
    EntryOpd,
    /// A PRX export-table descriptor.
    ExportOpd,
    /// Validated descriptor found by the `.opd` region scan.
    OpdScan,
}

impl FunctionOrigin {
    /// Stable lowercase tag for output surfaces.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EntryOpd => "entry-opd",
            Self::ExportOpd => "export-opd",
            Self::OpdScan => "opd-scan",
        }
    }
}

/// Why a function map could not be built.
#[derive(Debug, thiserror::Error)]
pub enum FuncMapError {
    /// Input shorter than an ELF header.
    #[error("funcmap: input shorter than an ELF header")]
    TooSmall,
    /// Program headers failed to parse.
    #[error("funcmap: {0}")]
    Elf(#[from] LoadError),
    /// PRX-specific structures failed to parse.
    #[error("funcmap: {0}")]
    Prx(#[from] PrxParseError),
    /// Neither a main executable nor a PS3 PRX.
    #[error("funcmap: e_type 0x{0:04x} is neither ET_EXEC nor PS3 PRX")]
    UnsupportedElfType(u16),
}

/// ELF e_type for a main executable.
const ET_EXEC: u16 = 2;

struct Anchor {
    opd_vaddr: u64,
    code: u32,
    name: FunctionName,
    origin: FunctionOrigin,
}

/// Build the function map for a decrypted main ELF or PRX image.
///
/// # Errors
///
/// Fails on malformed ELF / PRX structure; an image with no valid
/// OPD anchors yields an empty map, not an error.
pub fn build(data: &[u8]) -> Result<FunctionMap, FuncMapError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(FuncMapError::TooSmall);
    }
    let e_type = u16::from_be_bytes([data[16], data[17]]);
    // pt_load_segments rejects non-ELF64 / non-big-endian images
    // (EI_CLASS / EI_DATA checks), so the raw e_type and e_entry
    // reads below only ever see the ELF64-BE layout.
    let segments = pt_load_segments(data)?;

    let mut anchors: Vec<Anchor> = Vec::new();
    let mut toc_set: BTreeSet<u32> = BTreeSet::new();

    match e_type {
        ET_EXEC => {
            let e_entry = u64::from_be_bytes(data[24..32].try_into().expect("8 bytes"));
            if let Some((code, toc)) = deref_opd(data, &segments, e_entry) {
                // Corrupt-OPD rule carried over from the PRX parser:
                // toc == 0 rejects; code == 0 is legal.
                if toc != 0 {
                    toc_set.insert(toc);
                    anchors.push(Anchor {
                        opd_vaddr: e_entry,
                        code,
                        name: FunctionName::Known("entry"),
                        origin: FunctionOrigin::EntryOpd,
                    });
                }
            }
        }
        ET_PRX => {
            let prx = parse_prx(data)?;
            // No toc == 0 guard here: `find_system_opd` already
            // returns None for corrupt descriptors, so a populated
            // `module_start` / `module_stop` carries a nonzero toc.
            for (opd, name) in [
                (prx.module_start, "module_start"),
                (prx.module_stop, "module_stop"),
            ] {
                if let Some(opd) = opd {
                    toc_set.insert(opd.toc);
                    anchors.push(Anchor {
                        opd_vaddr: opd.opd_vaddr as u64,
                        code: opd.code,
                        name: FunctionName::Known(name),
                        origin: FunctionOrigin::ExportOpd,
                    });
                }
            }
            for lib in &prx.exports {
                for export in &lib.functions {
                    let Some((code, toc)) = deref_opd(data, &segments, export.vaddr as u64) else {
                        continue;
                    };
                    if toc == 0 {
                        continue;
                    }
                    toc_set.insert(toc);
                    anchors.push(Anchor {
                        opd_vaddr: export.vaddr as u64,
                        code,
                        name: FunctionName::Nid(export.nid),
                        origin: FunctionOrigin::ExportOpd,
                    });
                }
            }
        }
        other => return Err(FuncMapError::UnsupportedElfType(other)),
    }

    // First writer wins: anchors carry names, scan hits do not.
    let mut starts: BTreeMap<u32, (FunctionName, FunctionOrigin)> = BTreeMap::new();
    for anchor in &anchors {
        if code_in_exec(&segments, anchor.code) {
            starts
                .entry(anchor.code)
                .or_insert((anchor.name, anchor.origin));
        }
    }

    let mut truncated = false;
    let mut insert_scanned = |starts: &mut BTreeMap<u32, (FunctionName, FunctionOrigin)>,
                              code: u32| {
        if starts.len() >= MAX_FUNCTIONS {
            truncated = true;
            return false;
        }
        starts
            .entry(code)
            .or_insert((FunctionName::Synthetic, FunctionOrigin::OpdScan));
        true
    };

    // Grow scan: the `.opd` data is a contiguous run of descriptors;
    // seed at each anchor and extend in 8-byte steps while entries
    // validate.
    for anchor in &anchors {
        for step in [8i64, -8] {
            let mut vaddr = anchor.opd_vaddr;
            loop {
                vaddr = vaddr.wrapping_add(step as u64);
                let Some(code) = validate_descriptor(data, &segments, &toc_set, vaddr) else {
                    break;
                };
                if !insert_scanned(&mut starts, code) {
                    break;
                }
            }
        }
    }

    // Fallback sweep over all non-executable file-backed bytes;
    // the TOC-set check does the false-positive filtering.
    'sweep: for seg in segments.iter().filter(|s| !s.executable) {
        let mut vaddr = seg.vaddr;
        let seg_end = seg.vaddr.saturating_add(seg.filesz);
        while vaddr.saturating_add(8) <= seg_end {
            if let Some(code) = validate_descriptor(data, &segments, &toc_set, vaddr) {
                if !insert_scanned(&mut starts, code) {
                    break 'sweep;
                }
            }
            vaddr += 4;
        }
    }

    // Spans: end = next start, clamped to the containing executable
    // segment's file-backed range.
    let ordered: Vec<(u32, FunctionName, FunctionOrigin)> = starts
        .into_iter()
        .map(|(start, (name, origin))| (start, name, origin))
        .collect();
    let mut functions = Vec::with_capacity(ordered.len());
    for (i, &(start, name, origin)) in ordered.iter().enumerate() {
        let seg_end = segments
            .iter()
            .find(|s| {
                s.executable
                    && (start as u64) >= s.vaddr
                    && (start as u64) < s.vaddr.saturating_add(s.filesz)
            })
            .map(|s| s.vaddr.saturating_add(s.filesz).min(u32::MAX as u64) as u32)
            .expect("starts are validated against executable segments");
        let end = match ordered.get(i + 1) {
            Some(&(next, _, _)) if next < seg_end => next,
            _ => seg_end,
        };
        functions.push(FunctionSpan {
            start,
            end,
            name,
            origin,
        });
    }

    Ok(FunctionMap {
        functions,
        truncated,
    })
}

/// Read the `{ code, toc }` pair at `vaddr` if the full 8 bytes are
/// file-backed by one segment. Every dereference bounds-checks
/// against `data.len()`; `pt_load_segments` does not validate
/// offset sums.
fn deref_opd(data: &[u8], segments: &[LoadSegment], vaddr: u64) -> Option<(u32, u32)> {
    let seg = segments.iter().find(|s| {
        vaddr >= s.vaddr
            && vaddr
                .checked_add(8)
                .is_some_and(|e| e <= s.vaddr.saturating_add(s.filesz))
    })?;
    let off = usize::try_from(seg.file_offset.checked_add(vaddr - seg.vaddr)?).ok()?;
    if off.checked_add(8)? > data.len() {
        return None;
    }
    let code = u32::from_be_bytes(data[off..off + 4].try_into().expect("4 bytes"));
    let toc = u32::from_be_bytes(data[off + 4..off + 8].try_into().expect("4 bytes"));
    Some((code, toc))
}

/// Whether `code` is a plausible function start: 4-aligned and
/// inside an executable segment's file-backed range.
fn code_in_exec(segments: &[LoadSegment], code: u32) -> bool {
    code.is_multiple_of(4)
        && segments.iter().any(|s| {
            s.executable
                && (code as u64) >= s.vaddr
                && (code as u64) < s.vaddr.saturating_add(s.filesz)
        })
}

/// Validate a descriptor candidate at `vaddr`, returning its code
/// address. The `toc == 0` corrupt-descriptor rule applies exactly
/// as in the PRX export walk.
fn validate_descriptor(
    data: &[u8],
    segments: &[LoadSegment],
    toc_set: &BTreeSet<u32>,
    vaddr: u64,
) -> Option<u32> {
    let (code, toc) = deref_opd(data, segments, vaddr)?;
    (toc != 0 && toc_set.contains(&toc) && code_in_exec(segments, code)).then_some(code)
}

#[cfg(test)]
#[path = "tests/funcmap_tests.rs"]
mod tests;
