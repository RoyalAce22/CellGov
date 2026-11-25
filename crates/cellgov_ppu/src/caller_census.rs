//! Finds LV2 syscall and LV1 hypercall sites in a PPU ELF, and groups
//! one module's syscall sites the way the caller tables record them.
//!
//! A hypercall site that resolves to N states one fact. On each path
//! that the scan models from an entry it records to the `sc`, r11 holds
//! N at the `sc`. The fact rests on these assumptions:
//! - the indirect-entry bound holds each word that control can enter
//!   other than by a fall-through or a direct branch. The word after a
//!   branch that links is an exception. The walk ends at that word,
//!   because the routine that the branch calls can change r11;
//! - the scan reads the effect of each word as the executor does,
//!   invalid forms included;
//! - an `sc` service can change any GPR and returns to the next word.
//!   The scan also keeps CTR and LR across an `sc`, and lv1 breaks this
//!   assumption. Its return from each hypercall in its table puts the
//!   handler in CTR [OFW-4.93 lv1 a:0x0F80] and an address in its own
//!   code in LR [OFW-4.93 lv1 a:0x1050]. So a path through an earlier
//!   `sc` can bring a CTR or LR that the scan does not hold;
//! - an interrupt enters only at its vector, and its handler returns
//!   with the CTR, LR and GPRs that the interrupt found.
//!
//! When the scan cannot prove the fact, the site stays unresolved. A
//! kernel can then hold the calls of a number only at unresolved sites.
//! That loss is correct behaviour. These are defects:
//! - a resolved site that a modelled path reaches with r11 unset or
//!   other than N;
//! - a word outside the bound that control can enter other than by a
//!   fall-through or a direct branch, when the word does not follow a
//!   branch that links;
//! - a flow that drops a constant of CTR, LR or a GPR without its lost
//!   mark.
//!
//! The scan does not state that a resolved site is reachable. It states
//! nothing about paths that no recorded entry reaches, or about code
//! outside the scanned segments.

use std::collections::{BTreeMap, BTreeSet};

use crate::funcmap::{self, FuncMapError, FunctionName};
use crate::hypercall_cfg::{Code, CodeSegment};
use crate::indirect_entry::{indirect_entries, real_mode_entries, FileSegment, LostOrigins};
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
///
/// A site is a word that is exactly `sc 1`
/// ([`PPC_SC_1`](cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1)), and its
/// `ordinal` is the hypercall number. A word with a nonzero reserved
/// field also decodes as `sc 1`, but such a word is data, so it is not
/// a site.
///
/// A site resolves when every path to it loads the same non-negative
/// constant into r11 with `li r11`. A path follows the direct branches
/// and fall-throughs of all the executable segments, and a direct
/// branch may cross segments. A branch may run in real mode when control
/// can reach it from one of these:
/// - the ELF entry or an interrupt vector;
/// - an `rfid`, `mtmsr` or `mtmsrd` that can clear MSR\[IR\].
///
/// The path to the branch may cross an indirect branch whose CTR or LR
/// the code builds as a constant. When an `rfid` can clear MSR\[IR\] at
/// an SRR0 that the scan does not know, every branch may run in real
/// mode. An absolute branch that may run in real mode also reaches the
/// word at the real address of its target. A site stays unresolved when
/// control can enter a word between the load and `sc` in one of these
/// other ways:
/// - the ELF entry;
/// - an interrupt vector;
/// - an indirect branch to an address that the image names, or to the
///   constant that the code builds in its CTR or LR with its bits 62 and
///   63 set to 0;
/// - an indirect branch that may run in real mode, to the word at the
///   real address of an address that the image names, of a constant that
///   the code builds in its CTR or LR, or of the word after a branch that
///   links;
/// - an indirect branch that may run in 32-bit mode, to the word at the
///   low-order 32 bits of each of those addresses. A branch may run in
///   32-bit mode when control can reach it from an `rfid` or `mtmsrd`
///   that can clear MSR\[SF\] and MSR bit 1;
/// - an `rfid` that can clear MSR\[SF\] and MSR bit 1 at an SRR0 that the
///   scan does not know, to the word at the low-order 32 bits of each of
///   those addresses;
/// - an `rfid`, `mtmsr` or `mtmsrd` that can clear MSR\[IR\], to the word
///   at the real address where it puts control;
/// - an `rfid` that can clear MSR\[IR\] at an SRR0 that the scan does not
///   know, to the word at the real address of each address that the
///   image names;
/// - a `bcctr`, `bclr` or `rfid` that can reach an address that the scan
///   does not know, which can enter any word.
///
/// The segments may lie anywhere in the 64-bit address space, as a
/// kernel's do.
///
/// # Errors
///
/// Returns [`CallerScanError`] when:
/// - the ELF or one of its segments is malformed;
/// - a relative jump table runs past the most entries the scan reads
///   from one table.
pub fn scan_hypercalls(elf: &[u8]) -> Result<Vec<CallerSite>, CallerScanError> {
    let segments = pt_load_segments(elf)?;
    let code = Code::new(executable_segments(
        elf,
        &segments,
        validate_segment_extent,
    )?);
    let files = file_segments(elf, &segments)?;
    let bound = indirect_entries(elf, &files, &code)?;
    let mut sites = crate::hypercall_cfg::scan(&code, &bound, &real_mode_entries(elf, &code));
    sites.sort_by_key(|site| site.address);
    Ok(sites)
}

/// A register whose constants a join of the hypercall scan drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JoinRegister {
    /// The GPR with this number.
    Gpr(u8),
    /// The link register.
    Lr,
}

/// A branch target where the hypercall scan dropped constants of one
/// register, because the paths to it brought more than it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JoinDrop {
    /// Guest address of the branch target.
    pub join: u64,
    /// The register whose constants the scan dropped.
    pub register: JoinRegister,
    /// The number of different constants that the paths brought to the
    /// register there, or `None` when it is more than 64. A constant of
    /// LR counts once for each word that wrote it.
    pub brought: Option<usize>,
}

/// A word that takes a value whose constants the hypercall scan dropped:
/// an `mtctr`, a `bclr` or an `rfid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedValue {
    /// Guest address of the word.
    pub address: u64,
    /// The branch targets that dropped constants of the value, by
    /// address.
    pub drops: Vec<JoinDrop>,
    /// Whether the value can also hold a constant that a branch target
    /// outside `drops` dropped.
    pub more: bool,
}

/// Why the hypercall scan of an ELF leaves every site unresolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HypercallLosses {
    /// Guest address of each `bcctr`, `bclr` or `rfid` that can reach an
    /// address that the scan does not know. When this list is not empty,
    /// [`scan_hypercalls`] leaves every site unresolved.
    pub lost_targets: Vec<u64>,
    /// Each word that takes a value whose constants the scan dropped,
    /// by address. The list is empty when `lost_targets` is empty.
    pub dropped: Vec<DroppedValue>,
}

/// Find each `bcctr`, `bclr` or `rfid` whose target the LV1 hypercall
/// scan of `elf` cannot bound, and the branch targets where the scan
/// dropped the constants that it can reach.
///
/// The scan is the one that [`scan_hypercalls`] runs.
///
/// # Errors
///
/// Returns [`CallerScanError`] in the cases that [`scan_hypercalls`]
/// does.
pub fn hypercall_losses(elf: &[u8]) -> Result<HypercallLosses, CallerScanError> {
    let segments = pt_load_segments(elf)?;
    let code = Code::new(executable_segments(
        elf,
        &segments,
        validate_segment_extent,
    )?);
    let files = file_segments(elf, &segments)?;
    let bound = indirect_entries(elf, &files, &code)?;
    let dropped = bound
        .lost_origins
        .into_iter()
        .map(|(address, LostOrigins { drops, more })| DroppedValue {
            address,
            drops: drops.into_iter().collect(),
            more,
        })
        .collect();
    Ok(HypercallLosses {
        lost_targets: bound.lost_targets.into_iter().collect(),
        dropped,
    })
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

/// The GPRs that the decoded word `decoded` writes, as a mask with bit
/// `n` set for GPR `n`.
///
/// The mask holds only the GPRs that the word writes, where
/// [`maybe_writes_gpr`] holds each GPR that a register field names.
/// The mask holds every GPR for these words:
/// - `lswx`, which reads its byte count from XER at run time;
/// - `sc`, whose handler can write any GPR;
/// - a fused pair, which decode does not produce.
///
/// A word that the PPE refuses as illegal keeps the answer of
/// [`maybe_writes_gpr`], because its error handler can emulate it. An
/// illegal `lq` also writes RT and RT+1, which [`maybe_writes_gpr`]
/// does not hold.
pub(crate) fn written_gprs(decoded: &PpuInstruction) -> u32 {
    use PpuInstruction as I;
    match *decoded {
        // [PPC-Book1 p:33 s:3.3.2] A load writes RT, and its update form also writes the EA into RA.
        I::Lwz { rt, .. }
        | I::Lbz { rt, .. }
        | I::Lhz { rt, .. }
        | I::Lha { rt, .. }
        | I::Ld { rt, .. }
        | I::Lwa { rt, .. }
        | I::Lwzx { rt, .. }
        | I::Lbzx { rt, .. }
        | I::Ldx { rt, .. }
        | I::Lhzx { rt, .. }
        | I::Lhax { rt, .. }
        | I::Lwax { rt, .. }
        | I::Ldbrx { rt, .. }
        | I::Lwbrx { rt, .. }
        | I::Lhbrx { rt, .. }
        | I::Ldarx { rt, .. }
        | I::Lwarx { rt, .. } => gpr_run(rt, 1),
        I::Lhau { rt, ra, .. }
        | I::Lwzu { rt, ra, .. }
        | I::Lbzu { rt, ra, .. }
        | I::Lhzu { rt, ra, .. }
        | I::Ldu { rt, ra, .. }
        | I::Lwzux { rt, ra, .. }
        | I::Lbzux { rt, ra, .. }
        | I::Lhzux { rt, ra, .. }
        | I::Ldux { rt, ra, .. }
        | I::Lhaux { rt, ra, .. }
        | I::Lwaux { rt, ra, .. } => gpr_run(rt, 1) | gpr_run(ra, 1),
        // [PPC-Book1 p:46 s:3.3.5] `lmw` loads GPRs RT through 31.
        I::Lmw { rt, .. } => gpr_run(rt, 32u32.saturating_sub(u32::from(rt))),
        // [PPC-Book1 p:48 s:3.3.6] `lswi` loads NB bytes, or 32 when NB is 0, into GPRs from RT up, four bytes to each GPR, with GPR 0 after GPR 31.
        I::Lswi { rt, nb, .. } => {
            let bytes = if nb == 0 { 32 } else { u32::from(nb) };
            gpr_run(rt, bytes.div_ceil(4))
        }
        // [PPC-Book1 p:40 s:3.3.3] A store writes no GPR, and its update form writes the EA into RA.
        I::Stw { .. }
        | I::Stb { .. }
        | I::Sth { .. }
        | I::Std { .. }
        | I::Stmw { .. }
        | I::Stwx { .. }
        | I::Stdx { .. }
        | I::Stbx { .. }
        | I::Sthx { .. }
        | I::Sdbrx { .. }
        | I::Stwbrx { .. }
        | I::Sthbrx { .. }
        | I::Stswi { .. }
        | I::Stswx { .. }
        | I::Stdcx { .. }
        | I::Stwcx { .. } => 0,
        I::Stwu { ra, .. }
        | I::Stdu { ra, .. }
        | I::Stbu { ra, .. }
        | I::Sthu { ra, .. }
        | I::Stdux { ra, .. }
        | I::Sthux { ra, .. }
        | I::Stwux { ra, .. }
        | I::Stbux { ra, .. } => gpr_run(ra, 1),
        // [PPC-Book1 p:105 s:4.6.2] A floating-point load writes FRT, and its update form also writes the EA into RA.
        // [PPC-Book1 p:107 s:4.6.3] A floating-point store with update writes the EA into RA.
        I::Lfs { .. }
        | I::Lfd { .. }
        | I::Lfsx { .. }
        | I::Lfdx { .. }
        | I::Stfs { .. }
        | I::Stfd { .. }
        | I::Stfiwx { .. }
        | I::Stfsx { .. }
        | I::Stfdx { .. }
        | I::Fp63 { .. }
        | I::Fp59 { .. } => 0,
        I::Lfsu { ra, .. }
        | I::Lfdu { ra, .. }
        | I::Lfsux { ra, .. }
        | I::Lfdux { ra, .. }
        | I::Stfsu { ra, .. }
        | I::Stfdu { ra, .. }
        | I::Stfsux { ra, .. }
        | I::Stfdux { ra, .. } => gpr_run(ra, 1),
        // [PPC-Book1 p:51 s:3.3.8] An arithmetic instruction places its result into RT.
        I::Addi { rt, .. }
        | I::Addis { rt, .. }
        | I::Subfic { rt, .. }
        | I::Mulli { rt, .. }
        | I::Addic { rt, .. }
        | I::AddicDot { rt, .. }
        | I::Add { rt, .. }
        | I::Subf { rt, .. }
        | I::Subfc { rt, .. }
        | I::Addc { rt, .. }
        | I::Subfe { rt, .. }
        | I::Neg { rt, .. }
        | I::Mullw { rt, .. }
        | I::Mulhwu { rt, .. }
        | I::Mulhw { rt, .. }
        | I::Mulhdu { rt, .. }
        | I::Mulhd { rt, .. }
        | I::Adde { rt, .. }
        | I::Addze { rt, .. }
        | I::Subfze { rt, .. }
        | I::Subfme { rt, .. }
        | I::Addme { rt, .. }
        | I::Mulld { rt, .. }
        | I::Divw { rt, .. }
        | I::Divwu { rt, .. }
        | I::Divd { rt, .. }
        | I::Divdu { rt, .. }
        | I::Li { rt, .. }
        | I::Lis { rt, .. } => gpr_run(rt, 1),
        // [PPC-Book1 p:65 s:3.3.11] A logical instruction places its result into RA.
        // [PPC-Book1 p:71 s:3.3.12] A rotate or shift instruction returns its result to a GPR, RA.
        I::Or { ra, .. }
        | I::Orc { ra, .. }
        | I::Ori { ra, .. }
        | I::Oris { ra, .. }
        | I::Xori { ra, .. }
        | I::Xoris { ra, .. }
        | I::And { ra, .. }
        | I::Andc { ra, .. }
        | I::Nor { ra, .. }
        | I::Xor { ra, .. }
        | I::Eqv { ra, .. }
        | I::Nand { ra, .. }
        | I::AndiDot { ra, .. }
        | I::AndisDot { ra, .. }
        | I::Extsh { ra, .. }
        | I::Extsb { ra, .. }
        | I::Extsw { ra, .. }
        | I::Cntlzw { ra, .. }
        | I::Cntlzd { ra, .. }
        | I::Popcntb { ra, .. }
        | I::Slw { ra, .. }
        | I::Srw { ra, .. }
        | I::Srawi { ra, .. }
        | I::Sraw { ra, .. }
        | I::Srad { ra, .. }
        | I::Sradi { ra, .. }
        | I::Sld { ra, .. }
        | I::Srd { ra, .. }
        | I::Rlwinm { ra, .. }
        | I::Rlwimi { ra, .. }
        | I::Rlwnm { ra, .. }
        | I::Rldicl { ra, .. }
        | I::Rldicr { ra, .. }
        | I::Rldic { ra, .. }
        | I::Rldimi { ra, .. }
        | I::Rldcl { ra, .. }
        | I::Rldcr { ra, .. }
        | I::Mr { ra, .. }
        | I::Slwi { ra, .. }
        | I::Srwi { ra, .. }
        | I::Clrlwi { ra, .. }
        | I::Clrldi { ra, .. }
        | I::Sldi { ra, .. }
        | I::Srdi { ra, .. }
        | I::RlwinmMask { ra, .. } => gpr_run(ra, 1),
        // [PPC-Book1 p:82 s:3.3.13] A move from a system register places it into RT.
        // [PPC-Book1 p:81 s:3.3.13] A move to a system register reads RS.
        I::Mftb { rt }
        | I::Mftbu { rt }
        | I::Mfcr { rt }
        | I::Mfocrf { rt, .. }
        | I::Mflr { rt }
        | I::Mfctr { rt }
        | I::Mfxer { rt }
        | I::Mfvrsave { rt }
        | I::Mfctrl { rt }
        | I::Mfsprg3 { rt } => gpr_run(rt, 1),
        I::Mtcrf { .. }
        | I::Mtocrf { .. }
        | I::Mtlr { .. }
        | I::Mtctr { .. }
        | I::Mtxer { .. }
        | I::Mtvrsave { .. }
        | I::Mcrxr { .. } => 0,
        I::Cmpwi { .. }
        | I::Cmplwi { .. }
        | I::Cmpdi { .. }
        | I::Cmpldi { .. }
        | I::Cmpw { .. }
        | I::Cmplw { .. }
        | I::Cmpd { .. }
        | I::Cmpld { .. }
        | I::CmpwZero { .. }
        | I::CmpdZero { .. }
        | I::Tw { .. }
        | I::Td { .. }
        | I::Twi { .. }
        | I::Tdi { .. }
        | I::B { .. }
        | I::Bc { .. }
        | I::Bclr { .. }
        | I::Bcctr { .. }
        | I::Bdnz { .. }
        | I::BcTrue { .. }
        | I::BcFalse { .. }
        | I::Blr { .. }
        | I::Mcrf { .. }
        | I::Crand { .. }
        | I::Crandc { .. }
        | I::Cror { .. }
        | I::Crorc { .. }
        | I::Crxor { .. }
        | I::Crnand { .. }
        | I::Crnor { .. }
        | I::Creqv { .. } => 0,
        I::Vx { .. }
        | I::Va { .. }
        | I::Vxor { .. }
        | I::Vsldoi { .. }
        | I::Mfvscr { .. }
        | I::Mtvscr { .. }
        | I::Lvlx { .. }
        | I::Lvrx { .. }
        | I::Lvlxl { .. }
        | I::Lvrxl { .. }
        | I::Stvlx { .. }
        | I::Stvrx { .. }
        | I::Stvlxl { .. }
        | I::Stvrxl { .. }
        | I::Lvsl { .. }
        | I::Lvebx { .. }
        | I::Lvsr { .. }
        | I::Lvehx { .. }
        | I::Lvewx { .. }
        | I::Lvx { .. }
        | I::Lvxl { .. }
        | I::Stvebx { .. }
        | I::Stvehx { .. }
        | I::Stvewx { .. }
        | I::Stvx { .. }
        | I::Stvxl { .. } => 0,
        I::Dcbz { .. }
        | I::Dcbst { .. }
        | I::Dcbf { .. }
        | I::Icbi { .. }
        | I::Dcbt { .. }
        | I::Dcbtst { .. }
        | I::Dst { .. }
        | I::Dstst { .. }
        | I::Dss { .. }
        | I::Sync { .. }
        | I::Eieio
        | I::Isync
        | I::Nop
        | I::Consumed => 0,
        I::PpePrivileged { raw } => privileged_written_gprs(raw),
        // [PPC-Book3 p:92 s:8.4] An `lq` with RT = RA invokes the illegal instruction error handler.
        I::PpeIllegal { raw } => (0..32).fold(privileged_written_gprs(raw), |mask, gpr| {
            if maybe_writes_gpr(raw, gpr) {
                mask | 1 << gpr
            } else {
                mask
            }
        }),
        I::Lswx { .. } | I::Sc { .. } | I::Fused(_) => u32::MAX,
    }
}

/// The GPRs that the word `decoded` writes on each run, as a mask with
/// bit `n` set for GPR `n`.
///
/// The mask holds no GPR for these words, which [`written_gprs`] takes to
/// write GPRs that they can leave unchanged:
/// - `sc`, whose handler can return each GPR unchanged;
/// - `lswx`, which loads no byte and leaves RT undefined when its byte
///   count in XER is 0;
/// - a divide, which leaves RT undefined when its divisor is 0 or its
///   quotient overflows;
/// - an illegal word, whose error handler need not emulate it;
/// - a fused pair, which decode does not produce.
///
/// An invalid form of a load or a store also takes the illegal-instruction
/// program interrupt. Its mask holds only the GPRs that the PPE loads
/// before it takes the interrupt.
///
/// [PPC-Book1 p:48 s:3.3.6] `lswx` loads the number of bytes that XER\[57:63\] gives, and with a count of 0 the contents of RT are undefined.
/// [PPC-Book1 p:58] For `divd` and `divw`, a divisor of 0 or a quotient that overflows leaves the contents of RT undefined.
/// [PPC-Book1 p:59] For `divdu` and `divwu`, a divisor of 0 leaves the contents of RT undefined.
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
pub(crate) fn surely_written_gprs(decoded: &PpuInstruction) -> u32 {
    use PpuInstruction as I;
    match *decoded {
        I::Sc { .. }
        | I::Lswx { .. }
        | I::Divw { .. }
        | I::Divwu { .. }
        | I::Divd { .. }
        | I::Divdu { .. }
        | I::PpeIllegal { .. }
        | I::Fused(_) => 0,
        _ => match crate::exec::invalid_form(decoded) {
            Some(form) => loaded_before_collision(decoded, form),
            None => written_gprs(decoded),
        },
    }
}

/// The GPRs that the invalid form `form` of `decoded` loads before the
/// collision, as a mask.
///
/// [PPC-Book1 p:46 s:3.3.5] `lmw` loads GPRs from RT up.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads GPRs from RT up, four bytes to each GPR, with GPR 0 after GPR 31.
fn loaded_before_collision(decoded: &PpuInstruction, form: crate::exec::InvalidForm) -> u32 {
    match *decoded {
        PpuInstruction::Lmw { rt, .. } | PpuInstruction::Lswi { rt, .. } => {
            let gprs = u32::try_from(form.ahead.div_ceil(4)).unwrap_or(u32::MAX);
            gpr_run(rt, gprs)
        }
        _ => 0,
    }
}

/// The GPRs that the privileged word `raw` writes, as a mask.
///
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book3 p:52 s:4.11] `slbmfev` and `slbmfee` place SLB entry fields into RT.
/// [PPC-Book3 p:92 s:8.4] `lq` loads the quadword into RT and RT+1.
fn privileged_written_gprs(raw: u32) -> u32 {
    let rt = ((raw >> 21) & 0x1f) as u8;
    match (raw >> 26, (raw >> 1) & 0x3ff) {
        (31, 83 | 339 | 851 | 915) => gpr_run(rt, 1),
        (56, _) => gpr_run(rt, 2),
        _ => 0,
    }
}

/// The mask of `count` GPRs from `first` up, with GPR 0 after GPR 31.
fn gpr_run(first: u8, count: u32) -> u32 {
    (0..count.min(32)).fold(0, |mask, n| mask | 1 << ((u32::from(first) + n) % 32))
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

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_mode_tests.rs"]
mod hypercall_real_mode_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_mode_indirect_tests.rs"]
mod hypercall_real_mode_indirect_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_mode_switch_tests.rs"]
mod hypercall_real_mode_switch_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_decrement_tests.rs"]
mod hypercall_ctr_decrement_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_call_tests.rs"]
mod hypercall_ctr_call_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_toc_candidate_tests.rs"]
mod hypercall_toc_candidate_tests;

#[cfg(test)]
#[path = "tests/caller_census_written_gprs_tests.rs"]
mod written_gprs_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_mode_join_tests.rs"]
mod hypercall_real_mode_join_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_join_tests.rs"]
mod hypercall_ctr_join_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_virtual_tests.rs"]
mod hypercall_ctr_virtual_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_rfid_low_bits_tests.rs"]
mod hypercall_rfid_low_bits_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_mode_alias_tests.rs"]
mod hypercall_real_mode_alias_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_rfid_real_mode_low_bits_tests.rs"]
mod hypercall_rfid_real_mode_low_bits_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_jump_table_low_bits_tests.rs"]
mod hypercall_jump_table_low_bits_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_copy_tests.rs"]
mod hypercall_ctr_copy_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_return_copy_tests.rs"]
mod hypercall_ctr_return_copy_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_stack_copy_tests.rs"]
mod hypercall_ctr_stack_copy_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_alias_constant_tests.rs"]
mod hypercall_real_alias_constant_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_alias_named_tests.rs"]
mod hypercall_real_alias_named_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_real_alias_word_tests.rs"]
mod hypercall_real_alias_word_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_sign_extended_word_tests.rs"]
mod hypercall_sign_extended_word_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_constant_join_tests.rs"]
mod hypercall_constant_join_tests;

#[cfg(test)]
#[path = "tests/caller_census_invalid_form_tests.rs"]
mod invalid_form_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_rfid_low_word_tests.rs"]
mod hypercall_rfid_low_word_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_mode32_branch_tests.rs"]
mod hypercall_mode32_branch_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_mode32_review_tests.rs"]
mod hypercall_mode32_review_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_rfid_lost_srr0_tests.rs"]
mod hypercall_rfid_lost_srr0_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_join_drop_tests.rs"]
mod hypercall_join_drop_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_jump_table_alias_tests.rs"]
mod hypercall_jump_table_alias_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_jump_table_entry_tests.rs"]
mod hypercall_jump_table_entry_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_reach_tests.rs"]
mod hypercall_reach_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_skip_return_tests.rs"]
mod hypercall_skip_return_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_stepped_lr_tests.rs"]
mod hypercall_stepped_lr_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_storage_lost_tests.rs"]
mod hypercall_storage_lost_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_toc_drop_tests.rs"]
mod hypercall_toc_drop_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_progression_tests.rs"]
mod hypercall_progression_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_progression_mode32_tests.rs"]
mod hypercall_progression_mode32_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_loaded_step_tests.rs"]
mod hypercall_loaded_step_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_named_data_branch_tests.rs"]
mod hypercall_named_data_branch_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_call_return_tests.rs"]
mod hypercall_call_return_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_unfollowed_write_tests.rs"]
mod hypercall_unfollowed_write_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_flag_lost_tests.rs"]
mod hypercall_flag_lost_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_flag_cell_tests.rs"]
mod hypercall_flag_cell_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_bcctr_return_copy_tests.rs"]
mod hypercall_ctr_bcctr_return_copy_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_memory_copy_tests.rs"]
mod hypercall_ctr_memory_copy_tests;

#[cfg(test)]
#[path = "tests/caller_census_string_slots_console_tests.rs"]
mod string_slots_console_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_unwritten_tests.rs"]
mod hypercall_ctr_unwritten_tests;

#[cfg(test)]
#[path = "tests/caller_census_hypercall_ctr_copy_unfollowed_tests.rs"]
mod hypercall_ctr_copy_unfollowed_tests;
