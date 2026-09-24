//! The ELF load path: the header check, memory sizing, and the copy of every PT_LOAD into guest memory.

use crate::state::PpuState;
use cellgov_mem::be::{read_u16, read_u64};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_EI_VERSION, ELF_E_MACHINE_OFFSET,
    ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ELF_PN_XNUM, EM_PPC64, EV_CURRENT,
};

use super::phdr::{checked_pt_loads, read_pt_loads};
use super::process_param::find_sys_process_param;

/// `(addr, size)` pair describing where a rejected segment would have
/// been placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPlacement {
    /// Guest address at which the segment would have started.
    pub addr: u64,
    /// In-memory size of the segment.
    pub size: u64,
}

/// Why loading failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// File too small to contain an ELF header, or program-header
    /// table arithmetic overflowed.
    #[error("PPU ELF too small for header")]
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    #[error("PPU ELF bad magic")]
    BadMagic,
    /// Not a 64-bit ELF (PPU ELFs must be ELF64).
    #[error("PPU ELF is not 64-bit")]
    Not64Bit,
    /// Not big-endian (PPU ELFs must be MSB).
    #[error("PPU ELF is not big-endian")]
    NotBigEndian,
    /// `EI_VERSION` is not `EV_CURRENT`.
    #[error("PPU ELF EI_VERSION 0x{ei_version:02x} is not EV_CURRENT ({EV_CURRENT})")]
    UnknownElfVersion {
        /// Declared `EI_VERSION`.
        ei_version: u8,
    },
    /// `e_machine` is not `EM_PPC64`.
    #[error("PPU ELF e_machine {e_machine} (0x{e_machine:04x}) is not EM_PPC64 ({EM_PPC64})")]
    NotPpc64 {
        /// Declared `e_machine`.
        e_machine: u16,
    },
    /// `e_phnum` is zero: the file has no program-header table, so
    /// nothing in it is loadable.
    #[error("PPU ELF declares no program header table (e_phnum=0); nothing in it is loadable")]
    NoProgramHeaders,
    /// `e_phnum` is `PN_XNUM`: the real count sits in section header 0,
    /// an extension this loader does not read.
    #[error("PPU ELF e_phnum=0x{ELF_PN_XNUM:04X} (PN_XNUM extension) is not supported")]
    PhdrCountExtended,
    /// A LOAD segment's file-backed bytes run past the end of the file.
    #[error(
        "PPU ELF LOAD segment[{segment_index}] at file offset 0x{file_offset:x} (size 0x{filesz:x}) runs past the file's 0x{file_len:x} bytes"
    )]
    SegmentTruncated {
        /// Index of the offending PT_LOAD in the program-header table.
        segment_index: usize,
        /// Declared `p_offset`.
        file_offset: u64,
        /// Declared `p_filesz`.
        filesz: u64,
        /// Length of the file.
        file_len: u64,
    },
    /// A LOAD segment's virtual address + size exceeds guest memory,
    /// overflows a 32-bit PS3 effective address, or arithmetic on the
    /// vaddr/memsz pair overflowed.
    #[error(
        "PPU ELF LOAD segment[{segment_index}] at 0x{:016x} (size 0x{:x}) out of range",
        placement.addr, placement.size
    )]
    SegmentOutOfRange {
        /// Where the segment would have been placed in guest memory.
        placement: SegmentPlacement,
        /// Index of the offending PT_LOAD in the program-header table.
        segment_index: usize,
    },
    /// A LOAD segment claims more file bytes than memory bytes
    /// (`p_filesz > p_memsz`), which the ELF format forbids for
    /// loadable segments.
    #[error("PPU ELF LOAD segment[{segment_index}] filesz 0x{filesz:x} exceeds memsz 0x{memsz:x}")]
    SegmentFileszExceedsMemsz {
        /// Index of the offending PT_LOAD in the program-header table.
        segment_index: usize,
        /// Declared file-image size.
        filesz: u64,
        /// Declared memory-image size.
        memsz: u64,
    },
    /// `e_phentsize` is not the ELF64 program-header size, so the
    /// table cannot be strided.
    #[error("PPU ELF declares program-header entry size {phentsize}, not {ELF_PHENTSIZE}")]
    BadPhentsize {
        /// Declared `e_phentsize`.
        phentsize: usize,
    },
}

pub(super) fn validate_load_segment_sizes(
    segment_index: usize,
    filesz: u64,
    memsz: u64,
) -> Result<(), LoadError> {
    if filesz > memsz {
        return Err(LoadError::SegmentFileszExceedsMemsz {
            segment_index,
            filesz,
            memsz,
        });
    }
    Ok(())
}

/// Entry point and the minimum guest memory size needed to hold every
/// PT_LOAD segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadResult {
    /// ELF entry point (set as state.pc).
    pub entry: u64,
    /// Minimum guest memory size to hold all segments.
    pub min_memory_size: usize,
    /// Guest-address range covering the loaded `sys_process_param_t`
    /// struct. `None` when the ELF carries no struct. Consumed by
    /// the cross-runner classifier as a non-semantic range.
    pub sys_proc_param_range: Option<core::ops::Range<u64>>,
}

/// Minimum guest memory needed to host every PT_LOAD (including BSS).
///
/// # Errors
///
/// Any [`read_pt_loads`] refusal, [`LoadError::SegmentFileszExceedsMemsz`],
/// and [`LoadError::SegmentOutOfRange`] for a segment whose end
/// overflows or lies above the 4 GiB effective-address ceiling. The
/// sizing reads headers only, so a segment's file bytes are not held
/// to the file.
pub fn required_memory_size(data: &[u8]) -> Result<usize, LoadError> {
    let mut max_addr: u64 = 0;
    for seg in read_pt_loads(data)? {
        validate_load_segment_sizes(seg.index, seg.filesz, seg.memsz)?;
        if seg.memsz == 0 {
            continue;
        }
        let out_of_range = LoadError::SegmentOutOfRange {
            placement: SegmentPlacement {
                addr: seg.vaddr,
                size: seg.memsz,
            },
            segment_index: seg.index,
        };
        let end = seg
            .vaddr
            .checked_add(seg.memsz)
            .ok_or(out_of_range.clone())?;
        if end > u64::from(u32::MAX) + 1 {
            return Err(out_of_range);
        }
        if end > max_addr {
            max_addr = end;
        }
    }
    Ok(max_addr as usize)
}

/// Load a PPU ELF64 into `memory` and set `state.pc` / `state.gpr[2]`
/// by dereferencing the entry-point OPD.
///
/// `.bss` (memsz > filesz) is zero-filled. If the entry address does
/// not point at a valid OPD the raw entry is written to `state.pc`
/// unchanged.
pub fn load_ppu_elf(
    data: &[u8],
    memory: &mut GuestMemory,
    state: &mut PpuState,
) -> Result<LoadResult, LoadError> {
    let segments = checked_pt_loads(data)?;
    let entry = read_u64(data, 24);

    let mem_size = memory.as_bytes().len();
    let mut max_addr: u64 = 0;
    // Address ranges this call actually committed. The entry
    // descriptor is only read from one of these: guest memory outside
    // them is zero-filled, and eight zero bytes there would otherwise
    // read back as a perfectly plausible descriptor naming pc 0.
    let mut loaded: Vec<std::ops::Range<u64>> = Vec::new();

    for seg in segments {
        let i = seg.index;
        let p_offset = seg.file_offset as usize;
        let p_vaddr = seg.vaddr;
        let p_filesz = seg.filesz;
        let p_memsz = seg.memsz;

        // `checked_pt_loads` refused p_filesz > p_memsz, which the ELF format
        // forbids for loadable segments: the bounds check below is
        // memsz-derived, so a header claiming extra file bytes would
        // route an oversized copy into apply_commit and panic instead
        // of erroring. It also refused file bytes past the end of the
        // file.
        if p_memsz == 0 {
            continue;
        }

        // PS3 effective addresses are 32-bit; reject anything that would
        // wrap on add or land above the 4 GiB EA ceiling before we cast
        // to usize for the apply_commit call below.
        let placement = SegmentPlacement {
            addr: p_vaddr,
            size: p_memsz,
        };
        let end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(LoadError::SegmentOutOfRange {
                placement,
                segment_index: i,
            })?;
        if end > u64::from(u32::MAX) + 1 || end > mem_size as u64 {
            return Err(LoadError::SegmentOutOfRange {
                placement,
                segment_index: i,
            });
        }

        let p_filesz_usz = p_filesz as usize;
        if p_filesz > 0 {
            let range = ByteRange::new(GuestAddr::new(p_vaddr), p_filesz).expect("valid range");
            memory
                .apply_commit(range, &data[p_offset..p_offset + p_filesz_usz])
                .expect("segment fits in memory");
        }

        if p_memsz > p_filesz {
            let bss_start = p_vaddr + p_filesz;
            let bss_size = p_memsz - p_filesz;
            let range = ByteRange::new(GuestAddr::new(bss_start), bss_size).expect("valid range");
            memory
                .apply_commit(range, &vec![0u8; bss_size as usize])
                .expect("BSS fits in memory");
        }

        if end > max_addr {
            max_addr = end;
        }
        loaded.push(p_vaddr..end);
    }

    // e_entry names a `function_descriptor` in .opd.
    use cellgov_ps3_abi::format::elf::function_descriptor;
    let entry_off = entry as usize;
    let mem_bytes = memory.as_bytes();
    // checked_add: a hostile e_entry near u64::MAX would wrap the
    // end-of-descriptor sum and either panic (debug) or index out of
    // bounds (release); an out-of-range entry takes the raw-entry
    // fallback below instead. The descriptor must also lie inside a
    // segment this call committed -- an e_entry that merely fits in
    // guest memory reads eight zero bytes and would set pc 0 with no
    // TOC, which is indistinguishable from a successful load.
    let descriptor_loaded = entry
        .checked_add(function_descriptor::SIZE as u64)
        .is_some_and(|end| loaded.iter().any(|r| r.start <= entry && end <= r.end));
    if descriptor_loaded
        && entry_off
            .checked_add(function_descriptor::SIZE)
            .is_some_and(|end| end <= mem_bytes.len())
    {
        let word_at = |field: usize| {
            let at = entry_off + field;
            u32::from_be_bytes([
                mem_bytes[at],
                mem_bytes[at + 1],
                mem_bytes[at + 2],
                mem_bytes[at + 3],
            ])
        };
        let code_addr = word_at(function_descriptor::CODE_OFFSET);
        let toc = word_at(function_descriptor::TOC_OFFSET);
        state.pc = code_addr as u64;
        state.set_gpr(2, toc as u64);
    } else {
        state.pc = entry;
    }

    // A wrapping end would invert the range into an empty one, and an
    // empty range silently classifies nothing -- the divergences this
    // range exists to attribute would fall through unlabelled. Drop the
    // range instead, so the classifier reports them as unattributed.
    let sys_proc_param_range = find_sys_process_param(data).and_then(|p| {
        let end = p.guest_addr.checked_add(u64::from(p.struct_size))?;
        Some(p.guest_addr..end)
    });

    Ok(LoadResult {
        entry,
        min_memory_size: max_addr as usize,
        sys_proc_param_range,
    })
}

/// Refuse a file that is not a big-endian ELF64 PPU object: too short
/// for a header, or the wrong magic, class, byte order, version or
/// machine.
pub(super) fn check_ppu_elf_header(data: &[u8]) -> Result<(), LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }
    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }
    if data[ELF_EI_CLASS] != ELFCLASS64 {
        return Err(LoadError::Not64Bit);
    }
    if data[ELF_EI_DATA] != ELFDATA2MSB {
        return Err(LoadError::NotBigEndian);
    }
    if data[ELF_EI_VERSION] != EV_CURRENT {
        return Err(LoadError::UnknownElfVersion {
            ei_version: data[ELF_EI_VERSION],
        });
    }
    let e_machine = read_u16(data, ELF_E_MACHINE_OFFSET);
    if e_machine != EM_PPC64 {
        return Err(LoadError::NotPpc64 { e_machine });
    }
    Ok(())
}
