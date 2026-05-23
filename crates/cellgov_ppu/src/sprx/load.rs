//! In-guest-memory PRX loader and PPC64 relocation applier.
//!
//! Consumes a [`super::ParsedPrx`] (produced by [`crate::sprx::parse_prx`])
//! and commits the module into guest memory as one atomic batch.

use std::collections::BTreeMap;

use super::{ParsedPrx, PrxRelocation, PrxSegment};

pub use cellgov_ps3_abi::elf::{
    R_PPC64_ADDR16_HA, R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO, R_PPC64_ADDR16_LO_DS, R_PPC64_ADDR32,
    R_PPC64_ADDR64, R_PPC64_REL24,
};

/// Relocation types `apply_relocations` knows how to apply.
///
/// Source of truth for "what the applier covers." Any new arm added
/// to the `match r.rtype` in `apply_relocations` must also land here;
/// the `applier_supported_types_match_apply_relocations` test enforces
/// alignment by feeding each entry through the applier and rejecting
/// `UnsupportedReloc`.
///
/// External consumers (the firmware reloc census regenerator at
/// `apps/cellgov_cli/tests/firmware_reloc_census.rs`) read this
/// slice rather than hardcoding a parallel list, so the regenerated
/// doc cannot disagree silently with the applier.
pub const APPLIER_SUPPORTED_TYPES: &[u32] = &[
    R_PPC64_ADDR32,
    R_PPC64_ADDR16_LO,
    R_PPC64_ADDR16_HI,
    R_PPC64_ADDR16_HA,
    R_PPC64_REL24,
    R_PPC64_ADDR64,
    R_PPC64_ADDR16_LO_DS,
];

/// `true` iff `apply_relocations` covers `rtype`.
pub const fn is_applier_supported(rtype: u32) -> bool {
    let mut i = 0;
    while i < APPLIER_SUPPORTED_TYPES.len() {
        if APPLIER_SUPPORTED_TYPES[i] == rtype {
            return true;
        }
        i += 1;
    }
    false
}

/// PRX module loaded into guest memory with relocations applied.
///
/// Every address is post-relocation (already includes `base`). `module_start`
/// and `module_stop` are derived from the parsed OPD plus `base` rather than
/// read back from guest memory: not every OPD field has a relocation entry,
/// so a post-relocation read is unreliable.
#[derive(Debug, Clone)]
pub struct LoadedPrx {
    /// Module name from `sys_prx_module_info_t`.
    pub name: String,
    /// Stable id derived from [`Self::name`] via FNV-1a-32.
    pub module_id: crate::prx_loader::PrxModuleId,
    /// Guest base at which the module was loaded.
    pub base: u64,
    /// Relocated TOC guest address.
    pub toc: u64,
    /// Text segment range `[text_start, text_end)`.
    pub text_start: u64,
    /// Exclusive end of the text segment range.
    pub text_end: u64,
    /// Data segment range `[data_start, data_end)`.
    pub data_start: u64,
    /// Exclusive end of the data segment range.
    pub data_end: u64,
    /// Exported function NIDs mapped to relocated OPD guest addresses.
    pub exports: BTreeMap<u32, u64>,
    /// Relocated `module_start` OPD, if exported.
    pub module_start: Option<LoadedOpd>,
    /// Relocated `module_stop` OPD, if exported.
    pub module_stop: Option<LoadedOpd>,
    /// Number of relocation entries applied.
    pub relocs_applied: usize,
}

/// Relocated OPD entry; both fields are absolute guest addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadedOpd {
    /// Function entry-point guest address.
    pub code: u64,
    /// TOC guest address paired with this entry point.
    pub toc: u64,
}

/// Failure mode while loading a parsed PRX into guest memory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrxLoadError {
    /// Segment does not fit in guest memory at the chosen base.
    /// `segment` names which of the PRX's two segments (typically
    /// `"text"` or `"data"`) failed.
    #[error(
        "PRX {segment} segment out of range at 0x{:016x} (size 0x{:x})",
        placement.addr, placement.size
    )]
    SegmentOutOfRange {
        /// Where the segment would have been placed in guest memory.
        placement: crate::loader::SegmentPlacement,
        /// Which PRX segment failed: `"text"` or `"data"`.
        segment: &'static str,
    },
    /// u64 overflow in segment-placement arithmetic. `cause`
    /// distinguishes the `base + vaddr` (start) computation from the
    /// `start + size` (end) computation. `segment` names which
    /// segment produced it. `size` is meaningful only when
    /// `cause = "start+size"`; for `cause = "base+vaddr"` size is
    /// reported as 0 because it wasn't involved.
    #[error("PRX {segment} segment overflow ({cause}) vaddr 0x{vaddr:016x} size 0x{size:x}")]
    SegmentSizeOverflow {
        /// Which segment produced the overflow.
        segment: &'static str,
        /// Which addition tripped: `"base+vaddr"` or `"start+size"`.
        cause: &'static str,
        /// Unrelocated PRX-space vaddr of the offending segment.
        vaddr: u64,
        /// The size field for end-overflow attribution; 0 when
        /// `cause = "base+vaddr"`.
        size: u64,
    },
    /// Text and data segments overlap in guest address space after
    /// relocation. The applier would clobber bytes of one with the
    /// other; surfacing rather than silently corrupting.
    #[error(
        "PRX segments overlap: first ends at 0x{first_end:016x}, second starts at 0x{second_start:016x}"
    )]
    SegmentOverlap {
        /// Computed end of the earlier segment (exclusive).
        first_end: u64,
        /// Computed start of the later segment.
        second_start: u64,
    },
    /// `ByteRange::new` rejected the (addr, length) pair (overflow,
    /// straddles a region boundary, etc.). Distinct from a region
    /// validation failure, which produces `MemoryFault`.
    #[error("PRX memory range invalid at 0x{addr:016x} length 0x{length:x}")]
    MemoryRangeInvalid {
        /// Guest address the rejected `ByteRange` started at.
        addr: u64,
        /// Length in bytes the rejected `ByteRange` requested.
        length: u64,
    },
    /// Per-write region check rejected the access. `source` is the
    /// underlying `MemError`; covers both reads and writes routed
    /// through `read_checked` / `apply_commit`.
    #[error("PRX memory fault at 0x{addr:016x}: {source}")]
    MemoryFault {
        /// Guest address the faulting access targeted.
        addr: u64,
        /// Underlying `MemError` from the region check.
        #[source]
        source: cellgov_mem::MemError,
    },
    /// Atomic-batch commit through `StagingMemory::drain_into`
    /// rejected the batch as a whole. Item-level attribution is not
    /// available at this layer; `count` is the number of staged
    /// writes the batch carried.
    #[error("PRX staging commit ({count} writes) rejected: {source}")]
    BatchCommitFailed {
        /// Number of staged writes in the rejected batch.
        count: usize,
        /// Underlying `MemError` reported by `drain_into`.
        #[source]
        source: cellgov_mem::MemError,
    },
    /// Relocation type code is not handled by the loader.
    #[error("PRX unsupported relocation type {0}")]
    UnsupportedReloc(u32),
    /// Relocation referenced a segment index outside the loaded
    /// `[text, data]` pair (>= 2). Indicates corruption or a firmware
    /// shape the loader does not yet model.
    #[error("PRX reloc segment {seg} out of range (sym 0x{sym:08x})")]
    RelocSegmentOutOfRange {
        /// Raw `sym` field carrying the offending segment index.
        sym: u32,
        /// Decoded segment index that was out of range.
        seg: usize,
    },
    /// Relocation `offset` falls outside its target segment's
    /// `memsz`. A malformed PRX could otherwise patch into a
    /// neighbouring segment.
    #[error("PRX reloc type {rtype} offset 0x{offset:x} out of segment (size 0x{seg_size:x})")]
    RelocOffsetOutOfSegment {
        /// Type of the offending relocation.
        rtype: u32,
        /// Patch offset within the target segment.
        offset: u64,
        /// `memsz` of the target segment the offset overflowed.
        seg_size: u64,
    },
    /// REL24 displacement exceeds the signed 26-bit range; or
    /// ADDR32 / ADDR16 wide variants where the computed value
    /// requires bits past the encoded width.
    #[error("PRX reloc type {rtype} overflow delta {delta}")]
    RelocOverflow {
        /// Type of the offending relocation.
        rtype: u32,
        /// Computed value or displacement (i64 for REL24, the value
        /// itself for ADDR32-class).
        delta: i64,
    },
    /// The patch offset, REL24 displacement, or ADDR16_LO_DS value
    /// has nonzero low bits the encoded field cannot represent.
    /// `kind` distinguishes the three sources; `value` carries the
    /// offending quantity in the form `kind` names.
    #[error("PRX reloc type {rtype} misaligned ({kind:?}) value {value}")]
    RelocMisaligned {
        /// Type of the offending relocation.
        rtype: u32,
        /// Pipeline stage that produced the misalignment.
        kind: RelocMisalignedKind,
        /// `PatchOffset`: `r.offset as i64`. `Displacement`: the
        /// computed REL24 delta. `EncodedValue`: the computed `S + A`.
        value: i64,
    },
}

/// Stage of the relocation pipeline that produced a misalignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocMisalignedKind {
    /// The patch offset within its target segment is not aligned
    /// to the relocation's write width. Fires before any encoding-
    /// specific check.
    PatchOffset,
    /// REL24 branch displacement (value - target) has nonzero low
    /// bits; the `LI || 0b00` encoding requires the displacement be
    /// 4-byte aligned.
    Displacement,
    /// ADDR16_LO_DS computed `S + A` has nonzero low bits; the
    /// DS-form encoding requires 4-byte alignment.
    EncodedValue,
}

/// Load a parsed PRX at `base` and apply relocations atomically.
///
/// Segment bytes, BSS zero-fill, and relocation patches stage into
/// one `StagingMemory` and commit via a single `drain_into`; any
/// failure (size overflow, overlap, region unavailable, faulting
/// reloc) leaves guest memory untouched.
///
/// `base` must be page-aligned and above the game's own footprint.
/// RMW relocations (REL24, ADDR16_LO_DS) resolve against the
/// parsed segment data, not guest memory, because segment bytes
/// have not yet committed.
pub fn load_prx(
    prx: &ParsedPrx,
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
) -> Result<LoadedPrx, PrxLoadError> {
    let (text_start, text_end) = segment_extent(base, &prx.text, "text")?;
    let (data_start, data_end) = segment_extent(base, &prx.data, "data")?;

    // Reject content (filesz) overlap. Real PS3 PRXes routinely
    // have text.memsz extending into BSS that data.vaddr overlaps
    // -- that's benign because data overwrites BSS zeros, not text
    // content. The dangerous case is text.filesz > data.vaddr,
    // where data overwrites actual instruction bytes; that's what
    // this check rejects. Symmetric form covers either ordering.
    let text_content_end =
        text_start
            .checked_add(prx.text.filesz)
            .ok_or(PrxLoadError::SegmentSizeOverflow {
                segment: "text",
                cause: "start+size",
                vaddr: prx.text.vaddr,
                size: prx.text.filesz,
            })?;
    let data_content_end =
        data_start
            .checked_add(prx.data.filesz)
            .ok_or(PrxLoadError::SegmentSizeOverflow {
                segment: "data",
                cause: "start+size",
                vaddr: prx.data.vaddr,
                size: prx.data.filesz,
            })?;
    if text_content_end > data_start && data_content_end > text_start {
        let (first_end, second_start) = if text_start <= data_start {
            (text_content_end, data_start)
        } else {
            (data_content_end, text_start)
        };
        return Err(PrxLoadError::SegmentOverlap {
            first_end,
            second_start,
        });
    }

    // Validate the target regions before any staging; this preserves
    // the SegmentOutOfRange diagnostic at segment granularity.
    validate_segment_region(memory, &prx.text, text_start, "text")?;
    validate_segment_region(memory, &prx.data, data_start, "data")?;

    let mut staging = cellgov_mem::StagingMemory::new();
    let relocs_applied = match stage_load(&mut staging, prx, base, text_start, data_start) {
        Ok(n) => n,
        Err(e) => {
            staging.clear();
            return Err(e);
        }
    };

    let count = staging.len();
    if let Err(source) = staging.drain_into(memory) {
        staging.clear();
        return Err(PrxLoadError::BatchCommitFailed { count, source });
    }

    let mut exports = BTreeMap::new();
    for lib in &prx.exports {
        for func in &lib.functions {
            exports.insert(func.nid, base + func.vaddr as u64);
        }
    }

    // opd.code/toc are absolute PRX vaddrs; add base only. Per-OPD toc is
    // honored rather than collapsed onto module_info.toc -- shipping firmware
    // keeps them equal but the OPD field is authoritative if it diverges.
    let module_start = prx.module_start.map(|opd| LoadedOpd {
        code: base + opd.code as u64,
        toc: base + opd.toc as u64,
    });
    let module_stop = prx.module_stop.map(|opd| LoadedOpd {
        code: base + opd.code as u64,
        toc: base + opd.toc as u64,
    });

    Ok(LoadedPrx {
        name: prx.name.clone(),
        module_id: prx.module_id,
        base,
        toc: base + prx.toc as u64,
        text_start,
        text_end,
        data_start,
        data_end,
        exports,
        module_start,
        module_stop,
        relocs_applied,
    })
}

/// Compute `(start, end)` for a segment in guest space, surfacing
/// any u64 overflow as `SegmentSizeOverflow`. End is exclusive.
/// `segment` names which segment for diagnostic attribution.
fn segment_extent(
    base: u64,
    seg: &PrxSegment,
    segment: &'static str,
) -> Result<(u64, u64), PrxLoadError> {
    let start = base
        .checked_add(seg.vaddr)
        .ok_or(PrxLoadError::SegmentSizeOverflow {
            segment,
            cause: "base+vaddr",
            vaddr: seg.vaddr,
            size: 0,
        })?;
    let end = start
        .checked_add(seg.memsz)
        .ok_or(PrxLoadError::SegmentSizeOverflow {
            segment,
            cause: "start+size",
            vaddr: seg.vaddr,
            size: seg.memsz,
        })?;
    Ok((start, end))
}

/// Region-availability check for a segment placement. The segment's
/// full `[guest_addr, guest_addr + memsz)` range must lie inside a
/// single region; `containing_region` returns `None` if it straddles
/// a boundary or falls outside the region map.
fn validate_segment_region(
    memory: &cellgov_mem::GuestMemory,
    seg: &PrxSegment,
    guest_addr: u64,
    segment: &'static str,
) -> Result<(), PrxLoadError> {
    if memory.containing_region(guest_addr, seg.memsz).is_none() {
        return Err(PrxLoadError::SegmentOutOfRange {
            placement: crate::loader::SegmentPlacement {
                addr: guest_addr,
                size: seg.memsz,
            },
            segment,
        });
    }
    Ok(())
}

/// Stage one segment's content bytes plus BSS zero-fill into the
/// shared staging buffer. Region availability is the caller's
/// responsibility (see [`validate_segment_region`]); this function
/// only constructs [`cellgov_mem::ByteRange`]s and pushes writes.
fn stage_segment(
    staging: &mut cellgov_mem::StagingMemory,
    seg: &PrxSegment,
    guest_addr: u64,
) -> Result<(), PrxLoadError> {
    // Parser invariant: extract_segment slices exactly filesz
    // bytes. A programmatic fixture that breaks it would miscompute
    // the BSS zero-fill width below.
    debug_assert_eq!(
        seg.data.len() as u64,
        seg.filesz,
        "PrxSegment.data.len() must equal filesz; parser enforces this, fixture violated"
    );
    if !seg.data.is_empty() {
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(guest_addr), seg.filesz)
                .ok_or(PrxLoadError::MemoryRangeInvalid {
                    addr: guest_addr,
                    length: seg.filesz,
                })?;
        staging.stage(cellgov_mem::StagedWrite {
            range,
            bytes: seg.data.clone(),
        });
    }
    let bss_size = seg.memsz.saturating_sub(seg.filesz);
    if bss_size > 0 {
        let bss_addr = guest_addr + seg.filesz;
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(bss_addr), bss_size)
            .ok_or(PrxLoadError::MemoryRangeInvalid {
                addr: bss_addr,
                length: bss_size,
            })?;
        staging.stage(cellgov_mem::StagedWrite {
            range,
            bytes: vec![0u8; bss_size as usize],
        });
    }
    Ok(())
}

/// Stage all writes for a full module load: segments first, then
/// relocations. Returns the relocation count on success. The
/// caller owns the [`cellgov_mem::StagingMemory`]; on `Err` the
/// caller must `clear()` it to satisfy the Drop precondition.
fn stage_load(
    staging: &mut cellgov_mem::StagingMemory,
    prx: &ParsedPrx,
    base: u64,
    text_start: u64,
    data_start: u64,
) -> Result<usize, PrxLoadError> {
    stage_segment(staging, &prx.text, text_start)?;
    stage_segment(staging, &prx.data, data_start)?;
    apply_relocations(staging, base, &prx.text, &prx.data, &prx.relocations)
}

/// Width in bytes of the patch a given relocation type writes.
/// `None` for any unsupported type; caller maps to
/// `PrxLoadError::UnsupportedReloc`.
fn reloc_write_size(rtype: u32) -> Option<u64> {
    match rtype {
        R_PPC64_ADDR32 | R_PPC64_REL24 => Some(4),
        R_PPC64_ADDR64 => Some(8),
        R_PPC64_ADDR16_LO | R_PPC64_ADDR16_LO_DS | R_PPC64_ADDR16_HI | R_PPC64_ADDR16_HA => Some(2),
        _ => None,
    }
}

/// Stage every relocation into `staging` as one batch.
///
/// All validation happens before any write is pushed. RMW
/// relocations (REL24, LO_DS) read existing bytes from the parsed
/// segment data; reads past `filesz` return zero (BSS).
///
/// On `Err` the caller must `clear()` the staging buffer to satisfy
/// the [`cellgov_mem::StagingMemory`] Drop precondition.
///
/// # Sequencing
///
/// Overlapping RMW relocations both see pre-batch content and stage
/// order picks the winner. The debug-only check below rejects
/// overlap rather than relying on last-staged-wins.
fn apply_relocations(
    staging: &mut cellgov_mem::StagingMemory,
    base: u64,
    text: &PrxSegment,
    data: &PrxSegment,
    relocs: &[PrxRelocation],
) -> Result<usize, PrxLoadError> {
    let segs: [&PrxSegment; 2] = [text, data];
    let seg_vaddrs = [text.vaddr, data.vaddr];
    let seg_sizes = [text.memsz, data.memsz];
    let mut staged_ranges: Vec<cellgov_mem::ByteRange> = Vec::with_capacity(relocs.len());
    for r in relocs {
        // PS3 PRX RELA r_sym packs target / value segment indices
        // in the low two bytes; bits 16:31 are unspecified and
        // ignored.
        let target_seg = (r.sym & 0xFF) as usize;
        let value_seg = ((r.sym >> 8) & 0xFF) as usize;

        if target_seg >= seg_vaddrs.len() {
            return Err(PrxLoadError::RelocSegmentOutOfRange {
                sym: r.sym,
                seg: target_seg,
            });
        }
        if value_seg >= seg_vaddrs.len() {
            return Err(PrxLoadError::RelocSegmentOutOfRange {
                sym: r.sym,
                seg: value_seg,
            });
        }
        let target_base = seg_vaddrs[target_seg];
        let value_base = seg_vaddrs[value_seg];
        let target_seg_size = seg_sizes[target_seg];

        // Reject unsupported types up front so the bound check has
        // a known write width.
        let write_size =
            reloc_write_size(r.rtype).ok_or(PrxLoadError::UnsupportedReloc(r.rtype))?;

        // [PPC-Book1 p:11 s:1.7] PowerPC instructions are 4-byte
        // aligned; ADDR16 halfwords and ADDR64 pointer-slots inherit
        // the natural alignment of their width. A misaligned patch
        // straddles two architectural slots.
        if r.offset & (write_size - 1) != 0 {
            return Err(PrxLoadError::RelocMisaligned {
                rtype: r.rtype,
                kind: RelocMisalignedKind::PatchOffset,
                value: r.offset as i64,
            });
        }

        // Offset + write_size must fit inside the target segment.
        // The spill case (aligned offset + write_size > memsz) is
        // only reachable with non-write-width-aligned memsz --
        // synthetic PRXes; real segments are page-aligned.
        let offset_end =
            r.offset
                .checked_add(write_size)
                .ok_or(PrxLoadError::RelocOffsetOutOfSegment {
                    rtype: r.rtype,
                    offset: r.offset,
                    seg_size: target_seg_size,
                })?;
        if offset_end > target_seg_size {
            return Err(PrxLoadError::RelocOffsetOutOfSegment {
                rtype: r.rtype,
                offset: r.offset,
                seg_size: target_seg_size,
            });
        }

        let target = base + target_base + r.offset;
        let value = (base + value_base).wrapping_add(r.addend as u64);

        // ADDR32 / ADDR16 family encode the bottom 32 bits of the
        // resolved value; `value >> 32 != 0` means the `as u32`
        // truncation below would silently drop bits 32..63.
        if matches!(
            r.rtype,
            R_PPC64_ADDR32
                | R_PPC64_ADDR16_LO
                | R_PPC64_ADDR16_LO_DS
                | R_PPC64_ADDR16_HI
                | R_PPC64_ADDR16_HA
        ) && value >> 32 != 0
        {
            return Err(PrxLoadError::RelocOverflow {
                rtype: r.rtype,
                delta: value as i64,
            });
        }
        // PRX ADDR16_{LO,HI,HA} are PPC32-style halves.
        let value32 = value as u32;

        let bytes: Vec<u8> = match r.rtype {
            R_PPC64_ADDR32 => value32.to_be_bytes().to_vec(),
            R_PPC64_ADDR64 => value.to_be_bytes().to_vec(),
            R_PPC64_ADDR16_LO => (value32 as u16).to_be_bytes().to_vec(),
            R_PPC64_ADDR16_LO_DS => {
                // ELFv1 PPC64 ABI: ADDR16_LO_DS computes `(S+A) &
                // 0xFFFC`; the encoded DS field requires the value
                // be 4-byte-aligned, so the low two bits of (S+A)
                // must be zero. A misaligned value is a corrupt
                // PRX; surface rather than silently zero.
                if value32 & 0x3 != 0 {
                    return Err(PrxLoadError::RelocMisaligned {
                        rtype: r.rtype,
                        kind: RelocMisalignedKind::EncodedValue,
                        value: value as i64,
                    });
                }
                // DS-form instructions reserve the low 2 bits of
                // the 16-bit halfword for the XO subfield; the
                // relocation must not disturb them.
                let existing = read_seg_u16(segs[target_seg], r.offset);
                let patched = ((value32 as u16) & 0xFFFC) | (existing & 0x0003);
                patched.to_be_bytes().to_vec()
            }
            R_PPC64_ADDR16_HI => ((value32 >> 16) as u16).to_be_bytes().to_vec(),
            R_PPC64_ADDR16_HA => {
                // +0x8000 cancels sign-extension of the paired LO.
                let ha = (value32.wrapping_add(0x8000) >> 16) as u16;
                ha.to_be_bytes().to_vec()
            }
            R_PPC64_REL24 => {
                // [PPC-Book1 p:8 s:1.7.1] I-form: OPCD | LI | AA |
                // LK with LI at IBM bits 6:29 (24 bits). The branch
                // target is EXTS(LI || 0b00), so the displacement
                // is forced 4-byte aligned; AA at IBM bit 30, LK at
                // IBM bit 31 must be preserved.
                let delta = (value as i64).wrapping_sub(target as i64);
                if delta & 0x3 != 0 {
                    return Err(PrxLoadError::RelocMisaligned {
                        rtype: r.rtype,
                        kind: RelocMisalignedKind::Displacement,
                        value: delta,
                    });
                }
                if !(-0x0200_0000..0x0200_0000).contains(&delta) {
                    return Err(PrxLoadError::RelocOverflow {
                        rtype: R_PPC64_REL24,
                        delta,
                    });
                }
                let mask: u32 = 0x03FF_FFFC;
                let insn = read_seg_u32(segs[target_seg], r.offset);
                let patched = (insn & !mask) | ((delta as u32) & mask);
                patched.to_be_bytes().to_vec()
            }
            // `reloc_write_size` above already rejected unsupported
            // types via UnsupportedReloc, so this arm is unreachable
            // by construction; the panic guards a logic violation.
            other => unreachable!("reloc_write_size accepted type {other} but match arm missing"),
        };

        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(target), write_size)
            .ok_or(PrxLoadError::MemoryRangeInvalid {
                addr: target,
                length: write_size,
            })?;
        staged_ranges.push(range);
        staging.stage(cellgov_mem::StagedWrite { range, bytes });
    }

    // No-overlap precondition: PRX corpora don't produce
    // overlapping read-modify-write relocations. Scoped to the
    // reloc range list because segment writes legitimately overlap
    // with reloc patches in the full staging buffer.
    #[cfg(debug_assertions)]
    {
        for i in 0..staged_ranges.len() {
            for j in (i + 1)..staged_ranges.len() {
                debug_assert!(
                    !staged_ranges[i].overlaps(staged_ranges[j]),
                    "apply_relocations: staged reloc writes {i} and {j} overlap"
                );
            }
        }
    }

    Ok(relocs.len())
}

/// Read 4 bytes from a parsed PRX segment at `seg_offset`. Reads
/// past `filesz` return zero (BSS). Caller guarantees
/// `seg_offset + 4 <= memsz` via the bound check upstream.
fn read_seg_u32(seg: &PrxSegment, seg_offset: u64) -> u32 {
    let off = seg_offset as usize;
    let end = off.saturating_add(4);
    if end as u64 <= seg.filesz {
        u32::from_be_bytes([
            seg.data[off],
            seg.data[off + 1],
            seg.data[off + 2],
            seg.data[off + 3],
        ])
    } else {
        0
    }
}

/// Read 2 bytes from a parsed PRX segment at `seg_offset`. Reads
/// past `filesz` return zero (BSS). Caller guarantees
/// `seg_offset + 2 <= memsz` via the bound check upstream.
fn read_seg_u16(seg: &PrxSegment, seg_offset: u64) -> u16 {
    let off = seg_offset as usize;
    let end = off.saturating_add(2);
    if end as u64 <= seg.filesz {
        u16::from_be_bytes([seg.data[off], seg.data[off + 1]])
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprx::parse_prx;

    use crate::sprx::test_fixtures::make_test_prx;

    #[test]
    fn load_test_prx_segments() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mem_size = 0x2000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(mem_size);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.name, "testmod");
        assert_eq!(loaded.base, base);
        assert_eq!(loaded.toc, base + 0x200);
        assert_eq!(loaded.text_start, base);
        assert_eq!(loaded.text_end, base + 0x100);
        assert_eq!(loaded.data_start, base + 0x100);
        assert_eq!(loaded.data_end, base + 0x300);
    }

    #[test]
    fn load_test_prx_exports_relocated() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.exports.len(), 3);
        assert_eq!(loaded.exports[&0xAAAAAAAA], base + 0x40);
        assert_eq!(loaded.exports[&0xBBBBBBBB], base + 0x50);
        assert_eq!(loaded.exports[&0xCCCCCCCC], base + 0x60);
    }

    #[test]
    fn load_test_prx_module_start_relocated() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(ms.code, base + 0x10);
        assert_eq!(ms.toc, base + 0x200);

        let mstop = loaded.module_stop.expect("module_stop");
        assert_eq!(mstop.code, base + 0x20);
        assert_eq!(mstop.toc, base + 0x200);
    }

    #[test]
    fn load_test_prx_relocations_applied() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.relocs_applied, 3);

        // ADDR32 text->text: target base+0x50, value base+0x80.
        let addr = (base + 0x50) as usize;
        let val = u32::from_be_bytes([
            mem.as_bytes()[addr],
            mem.as_bytes()[addr + 1],
            mem.as_bytes()[addr + 2],
            mem.as_bytes()[addr + 3],
        ]);
        assert_eq!(val, 0x1000_0080, "ADDR32 text->text mismatch");

        // ADDR16_HA: value 0x1000_0200, HA = (value + 0x8000) >> 16.
        let addr2 = (base + 0x54) as usize;
        let val2 = u16::from_be_bytes([mem.as_bytes()[addr2], mem.as_bytes()[addr2 + 1]]);
        assert_eq!(val2, 0x1000, "ADDR16_HA mismatch");

        // ADDR32 data->text patches module_start OPD code field.
        let addr3 = (base + 0x1F0) as usize;
        let val3 = u32::from_be_bytes([
            mem.as_bytes()[addr3],
            mem.as_bytes()[addr3 + 1],
            mem.as_bytes()[addr3 + 2],
            mem.as_bytes()[addr3 + 3],
        ]);
        assert_eq!(val3, 0x1000_0010, "ADDR32 data->text (OPD) mismatch");
    }

    #[test]
    fn load_test_prx_addr16_lo_and_hi() {
        let mut data = make_test_prx();

        let ph2 = 64 + 112;
        data[ph2 + 32..ph2 + 40].copy_from_slice(&48u64.to_be_bytes());

        let rel0 = 0x3F0;
        data[rel0..rel0 + 8].copy_from_slice(&0x58u64.to_be_bytes());
        let r_info0: u64 = R_PPC64_ADDR16_LO as u64;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
        data[rel0 + 16..rel0 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

        let rel1 = rel0 + 24;
        data[rel1..rel1 + 8].copy_from_slice(&0x5Au64.to_be_bytes());
        let r_info1: u64 = R_PPC64_ADDR16_HI as u64;
        data[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
        data[rel1 + 16..rel1 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();
        assert_eq!(loaded.relocs_applied, 2);

        // value = 0x1000_0000 + 0x12345678 = 0x2234_5678.
        let addr_lo = (base + 0x58) as usize;
        let lo = u16::from_be_bytes([mem.as_bytes()[addr_lo], mem.as_bytes()[addr_lo + 1]]);
        assert_eq!(lo, 0x5678, "ADDR16_LO mismatch");

        let addr_hi = (base + 0x5A) as usize;
        let hi = u16::from_be_bytes([mem.as_bytes()[addr_hi], mem.as_bytes()[addr_hi + 1]]);
        assert_eq!(hi, 0x2234, "ADDR16_HI mismatch");
    }

    #[test]
    fn load_prx_rejects_out_of_range() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let mut mem = cellgov_mem::GuestMemory::new(0x100);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(
            result,
            Err(PrxLoadError::SegmentOutOfRange { .. })
        ));
    }

    #[test]
    fn load_prx_rejects_reloc_with_out_of_range_segment() {
        // value_seg = 0x02 against a 2-entry [text, data] table.
        let mut data = make_test_prx();
        let rel0 = 0x3F0;
        let r_info: u64 = (0x0200u64 << 32) | R_PPC64_ADDR32 as u64;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(
            result,
            Err(PrxLoadError::RelocSegmentOutOfRange { seg: 2, .. })
        ));
    }

    #[test]
    fn load_module_start_not_double_added_when_text_vaddr_nonzero() {
        let mut data = make_test_prx();
        let ph0 = 64;
        let new_text_vaddr: u64 = 0x1000;
        data[ph0 + 16..ph0 + 24].copy_from_slice(&new_text_vaddr.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.text.vaddr, 0x1000);
        assert_eq!(
            prx.module_start.expect("module_start").code,
            0x10,
            "OPD code is absolute PRX vaddr, not text-relative",
        );

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(
            ms.code,
            base + 0x10,
            "ms.code = base + opd.code, not base + text.vaddr + opd.code",
        );
    }

    #[test]
    fn load_uses_per_opd_toc_not_module_info_toc() {
        let mut data = make_test_prx();
        let opd_base = 0x2E0;
        let alt_toc: u32 = 0x300;
        data[opd_base + 4..opd_base + 8].copy_from_slice(&alt_toc.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.toc, 0x200);
        assert_eq!(prx.module_start.expect("module_start").toc, alt_toc);

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(ms.toc, base + alt_toc as u64);
        // module_stop's OPD still carries 0x200; divergence proves per-OPD.
        let mstop = loaded.module_stop.expect("module_stop");
        assert_eq!(mstop.toc, base + 0x200);
    }

    #[test]
    fn applier_supported_types_match_apply_relocations() {
        // Feed each type in APPLIER_SUPPORTED_TYPES through
        // apply_relocations and reject UnsupportedReloc. Other errors
        // (overflow, write failure) are fine -- absence of
        // UnsupportedReloc is the invariant.
        let text = PrxSegment {
            vaddr: 0,
            filesz: 0x100,
            memsz: 0x100,
            data: vec![0u8; 0x100],
        };
        let data = PrxSegment {
            vaddr: 0,
            filesz: 0x100,
            memsz: 0x100,
            data: vec![0u8; 0x100],
        };
        for &rtype in APPLIER_SUPPORTED_TYPES {
            let relocs = vec![PrxRelocation {
                offset: 0,
                rtype,
                sym: 0,
                addend: 0,
            }];
            let mut staging = cellgov_mem::StagingMemory::new();
            let result = apply_relocations(&mut staging, 0, &text, &data, &relocs);
            staging.clear();
            match result {
                Ok(_) => {}
                Err(PrxLoadError::UnsupportedReloc(t)) => panic!(
                    "type {t} listed in APPLIER_SUPPORTED_TYPES but apply_relocations has no match arm"
                ),
                Err(_) => {}
            }
        }
    }

    #[test]
    fn is_applier_supported_matches_const_list() {
        for &t in APPLIER_SUPPORTED_TYPES {
            assert!(
                is_applier_supported(t),
                "type {t} missing from is_applier_supported"
            );
        }
        assert!(!is_applier_supported(99), "type 99 is not covered");
        assert!(!is_applier_supported(0), "type 0 (NONE) is not covered");
    }

    #[test]
    fn load_prx_rejects_unsupported_reloc() {
        let mut data = make_test_prx();

        let rel0 = 0x3F0;
        let r_info: u64 = 99;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(result, Err(PrxLoadError::UnsupportedReloc(99))));
    }

    #[test]
    fn load_real_liblv2() {
        let path = std::path::PathBuf::from(
            "../../tools/rpcs3/dev_flash_decrypted/sys/external/liblv2.prx",
        );
        if !path.exists() {
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.name, "liblv2");
        assert_eq!(loaded.base, base);
        assert_eq!(loaded.toc, base + 0x1c620);
        assert!(loaded.relocs_applied > 1000);

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(ms.code, base, "module_start code should be at base");
        assert_eq!(ms.toc, base + 0x1c620, "module_start TOC");

        let text_start = base as usize;
        let first_insn = u32::from_be_bytes([
            mem.as_bytes()[text_start],
            mem.as_bytes()[text_start + 1],
            mem.as_bytes()[text_start + 2],
            mem.as_bytes()[text_start + 3],
        ]);
        let opcode = first_insn >> 26;
        assert!(
            opcode > 0 && opcode < 64,
            "first instruction should be valid PPC64, got 0x{:08x}",
            first_insn
        );

        assert!(
            loaded
                .exports
                .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::INITIALIZE_TLS),
            "should export sys_initialize_tls"
        );
        assert!(
            loaded
                .exports
                .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::MALLOC),
            "should export _sys_malloc"
        );
    }
}
