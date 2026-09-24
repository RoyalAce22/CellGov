//! The program-header table, the PT_LOAD readers, and the virtual-address-to-file-offset lookup.

use cellgov_mem::be::{read_u16, read_u32, read_u64};
use cellgov_ps3_abi::format::elf::{
    ELF_PHENTSIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, ELF_PN_XNUM, PT_LOAD,
};

use super::load::{check_ppu_elf_header, validate_load_segment_sizes, LoadError, SegmentPlacement};

/// Byte offset of program-header slot `i`.
///
/// # Errors
///
/// [`LoadError::BadPhentsize`] when the header declares a slot size
/// other than the ELF64 program header, and [`LoadError::TooSmall`] on
/// overflow or when the slot would extend past `data_len`.
///
/// [CBE-Handbook p:392 s:14.2.1] a loader builds the process image
/// from the program header table, whose slot layout comes from the
/// base ELF definition that the PPE-ELF ABI extends -- so exactly one
/// slot size is valid here, not a range.
///
/// The check is a refusal rather than a clamp because `e_phentsize` is
/// attacker-supplied and every reader below indexes the slot at fixed
/// ELF64 offsets -- `p_memsz` at 40, `p_align` at 48. A narrower
/// declared slot puts those reads outside the validated window, and a
/// wider or zero one makes the stride disagree with the layout the
/// readers assume.
pub(super) fn ph_slot_base(
    data_len: usize,
    phoff: usize,
    phentsize: usize,
    i: usize,
) -> Result<usize, LoadError> {
    if phentsize != ELF_PHENTSIZE {
        return Err(LoadError::BadPhentsize { phentsize });
    }
    let prod = i.checked_mul(phentsize).ok_or(LoadError::TooSmall)?;
    let base = phoff.checked_add(prod).ok_or(LoadError::TooSmall)?;
    let end = base.checked_add(phentsize).ok_or(LoadError::TooSmall)?;
    if end > data_len {
        return Err(LoadError::TooSmall);
    }
    Ok(base)
}

/// A PT_LOAD segment's address range and permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    /// Index of the program header within the ELF.
    pub index: usize,
    /// `p_offset`: file-relative byte position where the segment's
    /// initialized bytes begin. Callers deriving a
    /// `[file_offset, file_offset + filesz)` range must validate the
    /// sum against the file length.
    pub file_offset: u64,
    /// Guest virtual address of the segment start.
    pub vaddr: u64,
    /// Bytes read from the ELF file (<= memsz).
    pub filesz: u64,
    /// Total size in memory, including BSS tail (>= filesz).
    pub memsz: u64,
    /// ELF p_flags bit 0 (PF_X: executable).
    pub executable: bool,
    /// ELF p_flags bit 1 (PF_W: writable).
    pub writable: bool,
    /// ELF p_flags bit 2 (PF_R: readable).
    pub readable: bool,
}

/// Where a PPU executable's program-header table lies in the file.
///
/// [`program_header_table`] is the only constructor, and it returns one
/// only after checking that every slot lies inside the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramHeaderTable {
    offset: u64,
    count: u16,
}

impl ProgramHeaderTable {
    /// `e_phoff`: file offset of slot 0.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// `e_phnum`: the number of slots, never 0 or `PN_XNUM`.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// One past the table's last byte, as a file offset. It is at most
    /// the file's length.
    pub fn end(&self) -> u64 {
        self.offset + u64::from(self.count) * ELF_PHENTSIZE as u64
    }

    /// File offset of slot `i`, for `i < count`.
    pub(super) fn slot(&self, i: usize) -> usize {
        self.offset as usize + i * ELF_PHENTSIZE
    }
}

/// The program-header table of a PPU executable.
///
/// The one reader of the table's extent: [`read_pt_loads`] walks the
/// slots it locates.
///
/// # Errors
///
/// - The header refusals: [`LoadError::TooSmall`],
///   [`LoadError::BadMagic`], [`LoadError::Not64Bit`],
///   [`LoadError::NotBigEndian`], [`LoadError::UnknownElfVersion`],
///   [`LoadError::NotPpc64`].
/// - [`LoadError::PhdrCountExtended`] and [`LoadError::NoProgramHeaders`]
///   for a count the table cannot be walked with.
/// - [`LoadError::BadPhentsize`], or [`LoadError::TooSmall`] for a table
///   running past the file.
pub fn program_header_table(data: &[u8]) -> Result<ProgramHeaderTable, LoadError> {
    check_ppu_elf_header(data)?;
    let offset = read_u64(data, ELF_PHOFF_OFFSET);
    let phentsize = usize::from(read_u16(data, ELF_PHENTSIZE_OFFSET));
    let count = read_u16(data, ELF_PHNUM_OFFSET);
    if count == ELF_PN_XNUM {
        return Err(LoadError::PhdrCountExtended);
    }
    // A file with no program-header table writes e_phnum = 0, and a
    // zero count locates no entries for e_phentsize to size, so the
    // count is read first: an absent table gets its own refusal.
    if count == 0 {
        return Err(LoadError::NoProgramHeaders);
    }
    // The last slot lying inside the file puts every earlier one there.
    // An offset past usize saturates, so the slot size is checked first
    // and the extent check then refuses it as TooSmall.
    let offset_usize = usize::try_from(offset).unwrap_or(usize::MAX);
    ph_slot_base(data.len(), offset_usize, phentsize, usize::from(count) - 1)?;
    Ok(ProgramHeaderTable { offset, count })
}

/// Every PT_LOAD program header, in program-header order, zero-sized
/// ones included, as the table declares them.
///
/// The one PT_LOAD reader: [`checked_pt_loads`] and
/// [`pt_load_segments`] read through it. It validates the header and
/// the table, not the segments: a segment may claim file bytes past the
/// end of the file, or end at the top of the address space. A caller
/// that copies or reads a segment's bytes takes [`checked_pt_loads`].
///
/// # Errors
///
/// Any [`program_header_table`] refusal.
pub fn read_pt_loads(data: &[u8]) -> Result<Vec<LoadSegment>, LoadError> {
    let table = program_header_table(data)?;
    let mut out = Vec::new();
    for i in 0..usize::from(table.count) {
        let base = table.slot(i);
        if read_u32(data, base) != PT_LOAD {
            continue;
        }
        let p_flags = read_u32(data, base + 4);
        let file_offset = read_u64(data, base + 8);
        let vaddr = read_u64(data, base + 16);
        let filesz = read_u64(data, base + 32);
        let memsz = read_u64(data, base + 40);
        out.push(LoadSegment {
            index: i,
            file_offset,
            vaddr,
            filesz,
            memsz,
            executable: (p_flags & 0x1) != 0,
            writable: (p_flags & 0x2) != 0,
            readable: (p_flags & 0x4) != 0,
        });
    }
    Ok(out)
}

/// [`read_pt_loads`], each segment checked against the file and the
/// address space.
///
/// A segment it returns satisfies `filesz <= memsz`, `vaddr + memsz`
/// fits u64, and, when `filesz > 0`, `[file_offset, file_offset +
/// filesz)` lies inside the file. [`load_ppu_elf`](super::load_ppu_elf) reads through it.
///
/// # Errors
///
/// Any [`read_pt_loads`] refusal, then per segment, in this order:
/// [`LoadError::SegmentFileszExceedsMemsz`],
/// [`LoadError::SegmentOutOfRange`] when `vaddr + memsz` overflows,
/// and [`LoadError::SegmentTruncated`].
pub fn checked_pt_loads(data: &[u8]) -> Result<Vec<LoadSegment>, LoadError> {
    let segments = read_pt_loads(data)?;
    for seg in &segments {
        validate_load_segment_sizes(seg.index, seg.filesz, seg.memsz)?;
        if seg.vaddr.checked_add(seg.memsz).is_none() {
            return Err(LoadError::SegmentOutOfRange {
                placement: SegmentPlacement {
                    addr: seg.vaddr,
                    size: seg.memsz,
                },
                segment_index: seg.index,
            });
        }
        if seg.filesz > 0
            && seg
                .file_offset
                .checked_add(seg.filesz)
                .is_none_or(|end| end > data.len() as u64)
        {
            return Err(LoadError::SegmentTruncated {
                segment_index: seg.index,
                file_offset: seg.file_offset,
                filesz: seg.filesz,
                file_len: data.len() as u64,
            });
        }
    }
    Ok(segments)
}

/// Enumerate PT_LOAD segments in program-header order (zero-sized
/// segments omitted). Matches the order `load_ppu_elf` copies them.
///
/// The segments are [`read_pt_loads`]'s, unchecked: see it for what a
/// segment may claim.
///
/// # Errors
///
/// Any [`read_pt_loads`] refusal.
pub fn pt_load_segments(data: &[u8]) -> Result<Vec<LoadSegment>, LoadError> {
    let mut segments = read_pt_loads(data)?;
    segments.retain(|s| s.memsz != 0);
    Ok(segments)
}

/// Where the bytes of a guest address range come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSource {
    /// The file backs the whole range.
    FileBacked {
        /// The segment chosen to back it.
        segment: LoadSegment,
        /// File offset of the range's first byte.
        file_offset: u64,
        /// How many segments' file-backed bytes hold the range; more
        /// than one when segments overlap.
        overlapping: usize,
    },
    /// No segment's file bytes hold the range, but a segment maps its
    /// first address past its file-backed bytes: zero-fill (BSS).
    ZeroFill {
        /// The first segment, in program-header order, that maps it.
        segment: LoadSegment,
    },
    /// No segment maps the address.
    Unmapped,
}

impl AddressSource {
    /// The file offset of the range, when the file backs it.
    #[must_use]
    pub fn file_offset(&self) -> Option<u64> {
        match self {
            Self::FileBacked { file_offset, .. } => Some(*file_offset),
            Self::ZeroFill { .. } | Self::Unmapped => None,
        }
    }
}

/// The file offset of the `len` bytes at `vaddr`, when the file backs
/// them; see [`address_source`].
pub(crate) fn file_offset_at(segments: &[LoadSegment], vaddr: u64, len: usize) -> Option<usize> {
    let offset = address_source(segments, vaddr, u64::try_from(len).ok()?).file_offset()?;
    usize::try_from(offset).ok()
}

/// Where the `len` bytes at `vaddr` come from, among `segments`.
///
/// The one virtual-address-to-file-offset lookup. Among the segments
/// whose file-backed bytes hold `[vaddr, vaddr + len)`, it takes the
/// smallest, then the lowest file offset, then the lowest address; in
/// a file whose segments do not overlap that is the only one.
#[must_use]
pub fn address_source(segments: &[LoadSegment], vaddr: u64, len: u64) -> AddressSource {
    let backing = |segment: &LoadSegment| -> Option<u64> {
        let delta = vaddr.checked_sub(segment.vaddr)?;
        if delta.checked_add(len)? > segment.filesz {
            return None;
        }
        segment.file_offset.checked_add(delta)
    };
    let mut candidates: Vec<(LoadSegment, u64)> = segments
        .iter()
        .filter_map(|s| backing(s).map(|offset| (*s, offset)))
        .collect();
    let overlapping = candidates.len();
    candidates.sort_by_key(|(s, _)| (s.filesz, s.file_offset, s.vaddr));
    if let Some(&(segment, file_offset)) = candidates.first() {
        return AddressSource::FileBacked {
            segment,
            file_offset,
            overlapping,
        };
    }
    let mapped = segments
        .iter()
        .find(|s| vaddr >= s.vaddr && s.vaddr.checked_add(s.memsz).is_some_and(|end| vaddr < end));
    match mapped {
        Some(segment) => AddressSource::ZeroFill { segment: *segment },
        None => AddressSource::Unmapped,
    }
}
