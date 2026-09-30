//! SPU ELF loader covering the subset used by PSL1GHT-compiled SPU
//! binaries (ELF32, big-endian, PT_LOAD segments only).

use crate::state::SpuState;
use cellgov_mem::be::{read_u16, read_u32};
use cellgov_ps3_abi::format::elf::{
    ELF32_E_ENTRY, ELF32_E_PHENTSIZE, ELF32_E_PHNUM, ELF32_E_PHOFF, ELF32_E_SHENTSIZE,
    ELF32_E_SHNUM, ELF32_E_SHOFF, ELF32_HEADER_SIZE, ELF32_PHDR_SIZE, ELF32_P_FLAGS,
    ELF_E_MACHINE_OFFSET, ELF_MAGIC, PT_LOAD,
};

/// Load failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// File is too small to contain an ELF header.
    #[error("SPU ELF too small for header")]
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    #[error("SPU ELF bad magic")]
    BadMagic,
    /// Not a 32-bit ELF.
    #[error("SPU ELF is not 32-bit")]
    Not32Bit,
    /// Not big-endian.
    #[error("SPU ELF is not big-endian")]
    NotBigEndian,
    /// A LOAD segment extends past the end of the file.
    #[error("SPU ELF LOAD segment truncated")]
    SegmentTruncated,
    /// A LOAD segment's virtual address + size exceeds local store.
    #[error("SPU ELF LOAD segment at vaddr 0x{vaddr:08x} (memsz {memsz}) exceeds local store")]
    SegmentOutOfRange {
        /// Virtual address of the segment.
        vaddr: u32,
        /// Memory size of the segment.
        memsz: u32,
    },
    /// A LOAD segment claims more file bytes than memory bytes
    /// (`p_filesz > p_memsz`), which the ELF format forbids for
    /// loadable segments.
    #[error("SPU ELF LOAD segment filesz 0x{filesz:08x} exceeds memsz 0x{memsz:08x}")]
    SegmentFileszExceedsMemsz {
        /// Declared file-image size.
        filesz: u32,
        /// Declared memory-image size.
        memsz: u32,
    },
    /// The ELF entry point leaves no whole instruction word inside
    /// local store.
    #[error("SPU ELF entry point 0x{entry:08x} is outside local store")]
    EntryOutOfRange {
        /// Declared `e_entry`.
        entry: u32,
    },
    /// `e_phentsize` is not the ELF32 program-header size, so the
    /// table cannot be strided.
    #[error("SPU ELF declares program-header entry size {phentsize}, not {ELF32_PHDR_SIZE}")]
    BadPhentsize {
        /// Declared `e_phentsize`.
        phentsize: usize,
    },
}

/// One PT_LOAD segment of an SPU ELF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuLoadSegment {
    /// Local-store address the segment loads at.
    pub vaddr: u32,
    /// File offset of the segment's bytes.
    pub offset: usize,
    /// Bytes the file holds.
    pub filesz: usize,
    /// Bytes the segment takes in local store.
    pub memsz: usize,
    /// The segment's `p_flags`.
    pub flags: u32,
}

impl SpuLoadSegment {
    /// The segment's file bytes within `data`, the ELF it came from.
    pub fn bytes<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[self.offset..self.offset + self.filesz]
    }
}

/// An SPU ELF's header and PT_LOAD segments, validated against a local
/// store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuElf {
    /// `e_machine`.
    pub machine: u16,
    /// `e_entry`.
    pub entry: u32,
    /// The PT_LOAD segments, in header order.
    pub segments: Vec<SpuLoadSegment>,
    /// Bytes from the header to the end of the last structure the file
    /// holds: the header, the program-header table, the segments' file
    /// bytes, and the section-header table when there is one. ELF names
    /// no total size, so this is where an embedded image ends.
    pub extent: usize,
}

/// Read an SPU ELF's header and PT_LOAD segments, checking each
/// against a local store of `ls_len` bytes.
///
/// # Errors
///
/// Returns [`LoadError`] on any header or segment validation failure.
pub fn parse_spu_elf(data: &[u8], ls_len: usize) -> Result<SpuElf, LoadError> {
    if data.len() < ELF32_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }

    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }

    // [CBE-Handbook p:393 s:14.2.2.1] SPE-ELF requires EI_CLASS=ELFCLASS32.
    if data[4] != 1 {
        return Err(LoadError::Not32Bit);
    }

    // [CBE-Handbook p:393 s:14.2.2.1] SPE-ELF requires EI_DATA=ELFDATA2MSB.
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }

    let machine = read_u16(data, ELF_E_MACHINE_OFFSET);
    let entry = read_u32(data, ELF32_E_ENTRY);
    let phoff = read_u32(data, ELF32_E_PHOFF) as usize;
    let phnum = read_u16(data, ELF32_E_PHNUM) as usize;
    let phentsize = read_u16(data, ELF32_E_PHENTSIZE) as usize;

    // [CBE-Handbook p:393 s:14.2.2.1 Table 14-1] an SPE-ELF object is
    // ELFCLASS32, so its program-header slot has exactly one architected
    // size. Every read below indexes the slot at fixed ELF32 offsets, so
    // a declared size other than that one is refused rather than strided
    // over: zero would re-read slot 0 `e_phnum` times and anything
    // narrower would overlap the neighbouring header.
    if phnum != 0 && phentsize != ELF32_PHDR_SIZE {
        return Err(LoadError::BadPhentsize { phentsize });
    }

    let mut segments = Vec::new();
    let mut extent = ELF32_HEADER_SIZE;
    if phnum != 0 {
        extent = extent.max(phoff.saturating_add(phnum * ELF32_PHDR_SIZE));
    }
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + ELF32_PHDR_SIZE > data.len() {
            return Err(LoadError::TooSmall);
        }

        let p_type = read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }

        let p_offset = read_u32(data, base + 4) as usize;
        let p_vaddr = read_u32(data, base + 8);
        let p_filesz = read_u32(data, base + 16) as usize;
        let p_memsz = read_u32(data, base + 20) as usize;
        let p_flags = read_u32(data, base + ELF32_P_FLAGS);

        // The local-store bound below is memsz-derived while the copy
        // is filesz bytes, so a header claiming extra file bytes would
        // index past the slice and panic instead of erroring.
        if p_filesz > p_memsz {
            return Err(LoadError::SegmentFileszExceedsMemsz {
                filesz: p_filesz as u32,
                memsz: p_memsz as u32,
            });
        }

        // [CBE-Handbook p:64 s:3.1.1] Local Store is 256 KB; segments must fit.
        let end = p_vaddr as usize + p_memsz;
        if end > ls_len {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz as u32,
            });
        }

        if p_offset + p_filesz > data.len() {
            return Err(LoadError::SegmentTruncated);
        }

        extent = extent.max(p_offset + p_filesz);
        segments.push(SpuLoadSegment {
            vaddr: p_vaddr,
            offset: p_offset,
            filesz: p_filesz,
            memsz: p_memsz,
            flags: p_flags,
        });
    }

    // [CBE-Handbook p:395 s:14.3 Table 14-4] an SPE-ELF image names the
    // LS size it targets, which is the SPU_LSLR setting it requires, so
    // a loadable image lies wholly inside that local store. The loader
    // rejects an entry point with no whole word left, rather than
    // deferring to the first failed fetch.
    if entry as usize + 4 > ls_len {
        return Err(LoadError::EntryOutOfRange { entry });
    }

    // An embedded image is usually stripped of its section headers, so
    // only a table that lies inside the data extends the image.
    let shoff = read_u32(data, ELF32_E_SHOFF) as usize;
    let shnum = read_u16(data, ELF32_E_SHNUM) as usize;
    let shentsize = read_u16(data, ELF32_E_SHENTSIZE) as usize;
    let sh_end = shoff.saturating_add(shnum.saturating_mul(shentsize));
    if shoff != 0 && shnum != 0 && sh_end <= data.len() {
        extent = extent.max(sh_end);
    }

    Ok(SpuElf {
        machine,
        entry,
        segments,
        extent,
    })
}

/// Load an SPU ELF binary into `state`, copying PT_LOAD segments into
/// LS, zeroing `.bss` (memsz > filesz), and setting `state.pc` to the
/// ELF entry point.
///
/// # Errors
///
/// Returns [`LoadError`] on any header or segment validation failure.
pub fn load_spu_elf(data: &[u8], state: &mut SpuState) -> Result<(), LoadError> {
    let elf = parse_spu_elf(data, state.ls.len())?;
    for segment in &elf.segments {
        let dst_start = segment.vaddr as usize;
        state.ls[dst_start..dst_start + segment.filesz].copy_from_slice(segment.bytes(data));
        state.ls[dst_start + segment.filesz..dst_start + segment.memsz].fill(0);
    }
    // [CBE-Handbook p:421 s:14.6.3.3] SPE loader transfers control to entry parameter (e_entry).
    state.pc = elf.entry;
    Ok(())
}

/// Load pre-laid-out `(ls_start, bytes)` segments into `state` and set
/// `state.pc` to `entry`; the user-image shape of `sys_spu_image`,
/// whose segments the caller already resolved.
///
/// # Errors
///
/// [`LoadError::SegmentOutOfRange`] when a segment ends past local
/// store, [`LoadError::EntryOutOfRange`] when `entry` leaves no whole
/// instruction word inside it.
pub fn load_ls_segments(
    segments: &[(u32, &[u8])],
    entry: u32,
    state: &mut SpuState,
) -> Result<(), LoadError> {
    for &(ls_start, bytes) in segments {
        let start = ls_start as usize;
        // [CBE-Handbook p:64 s:3.1.1] Local Store is 256 KB; segments must fit.
        let Some(end) = start
            .checked_add(bytes.len())
            .filter(|&e| e <= state.ls.len())
        else {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: ls_start,
                memsz: bytes.len() as u32,
            });
        };
        state.ls[start..end].copy_from_slice(bytes);
    }
    if entry as usize + 4 > state.ls.len() {
        return Err(LoadError::EntryOutOfRange { entry });
    }
    state.pc = entry;
    Ok(())
}

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/loader_segments_tests.rs"]
mod segments_tests;
