//! SPU ELF loader covering the subset used by PSL1GHT-compiled SPU
//! binaries (ELF32, big-endian, PT_LOAD segments only).

use crate::state::SpuState;
use cellgov_mem::be::{read_u16, read_u32};
use cellgov_ps3_abi::elf::{ELF32_HEADER_SIZE, ELF32_PHDR_SIZE, ELF_MAGIC, PT_LOAD};

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

/// Load an SPU ELF binary into `state`, copying PT_LOAD segments into
/// LS, zeroing `.bss` (memsz > filesz), and setting `state.pc` to the
/// ELF entry point.
///
/// # Errors
///
/// Returns [`LoadError`] on any header or segment validation failure.
pub fn load_spu_elf(data: &[u8], state: &mut SpuState) -> Result<(), LoadError> {
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

    let entry = read_u32(data, 24);
    let phoff = read_u32(data, 28) as usize;
    let phnum = read_u16(data, 44) as usize;
    let phentsize = read_u16(data, 42) as usize;

    // Every read below indexes the slot at fixed ELF32 offsets, so a
    // declared slot size other than the architected one is refused
    // rather than strided over: zero would re-read slot 0 `e_phnum`
    // times and anything narrower would overlap the neighbouring
    // header. RPCS3 `Loader/ELF.h` `elf_object::open` refuses the same
    // mismatch whenever `e_phnum` is nonzero.
    if phnum != 0 && phentsize != ELF32_PHDR_SIZE {
        return Err(LoadError::BadPhentsize { phentsize });
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
        if end > state.ls.len() {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz as u32,
            });
        }

        if p_offset + p_filesz > data.len() {
            return Err(LoadError::SegmentTruncated);
        }

        let dst_start = p_vaddr as usize;
        state.ls[dst_start..dst_start + p_filesz]
            .copy_from_slice(&data[p_offset..p_offset + p_filesz]);

        if p_memsz > p_filesz {
            let bss_start = dst_start + p_filesz;
            let bss_end = dst_start + p_memsz;
            state.ls[bss_start..bss_end].fill(0);
        }
    }

    // An entry point with no whole word left inside local store is
    // rejected at load rather than deferred to the first failed fetch.
    // RPCS3 `sys_spu.cpp` `sys_spu_thread_initialize` refuses a user
    // image whose entry point sits above `SPU_LS_SIZE - 4`.
    if entry as usize + 4 > state.ls.len() {
        return Err(LoadError::EntryOutOfRange { entry });
    }

    // [CBE-Handbook p:421 s:14.6.3.3] SPE loader transfers control to entry parameter (e_entry).
    state.pc = entry;
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
