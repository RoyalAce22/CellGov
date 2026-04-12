//! Minimal PPU ELF64 loader.
//!
//! Reads an ELF64 binary (PPU ELFs are 64-bit big-endian), extracts
//! LOAD program headers, and copies each segment into guest memory at
//! the specified virtual address. Sets the program counter to the ELF
//! entry point.
//!
//! This is a test loader, not a production ELF loader. It handles the
//! subset of ELF features that PSL1GHT-compiled PPU binaries use.

use crate::state::PpuState;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

/// Why loading failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// File is too small to contain an ELF header.
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    BadMagic,
    /// Not a 64-bit ELF (PPU ELFs must be ELF64).
    Not64Bit,
    /// Not big-endian (PPU ELFs must be MSB).
    NotBigEndian,
    /// A LOAD segment extends past the end of the file.
    SegmentTruncated,
    /// A LOAD segment's virtual address + size exceeds guest memory.
    SegmentOutOfRange {
        /// Virtual address of the segment.
        vaddr: u64,
        /// Memory size of the segment.
        memsz: u64,
    },
}

/// ELF64 header size.
const ELF_HEADER_SIZE: usize = 64;
/// ELF magic.
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
/// PT_LOAD segment type.
const PT_LOAD: u32 = 1;

/// Result of loading a PPU ELF: the entry point and the minimum guest
/// memory size needed to hold all LOAD segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadResult {
    /// ELF entry point (set as state.pc).
    pub entry: u64,
    /// Minimum guest memory size to hold all segments.
    pub min_memory_size: usize,
}

/// Load a PPU ELF64 binary into guest memory and set the PC.
///
/// Copies all PT_LOAD segments with nonzero memsz into guest memory
/// at the specified virtual addresses. The `.bss` portion
/// (memsz > filesz) is zeroed. Sets `state.pc` to the ELF entry
/// point.
pub fn load_ppu_elf(
    data: &[u8],
    memory: &mut GuestMemory,
    state: &mut PpuState,
) -> Result<LoadResult, LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }

    // Validate ELF magic
    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }

    // EI_CLASS must be 2 (64-bit)
    if data[4] != 2 {
        return Err(LoadError::Not64Bit);
    }

    // EI_DATA must be 2 (big-endian)
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }

    // Entry point: offset 24, 8 bytes BE
    let entry = read_u64(data, 24);

    // Program header table offset: offset 32, 8 bytes BE
    let phoff = read_u64(data, 32) as usize;

    // Program header entry size: offset 54, 2 bytes BE
    let phentsize = read_u16(data, 54) as usize;

    // Number of program headers: offset 56, 2 bytes BE
    let phnum = read_u16(data, 56) as usize;

    let mem_size = memory.as_bytes().len();
    let mut max_addr: usize = 0;

    // Process each program header
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            return Err(LoadError::TooSmall);
        }

        let p_type = read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }

        let p_offset = read_u64(data, base + 8) as usize;
        let p_vaddr = read_u64(data, base + 16);
        let p_filesz = read_u64(data, base + 32) as usize;
        let p_memsz = read_u64(data, base + 40) as usize;

        // Skip empty segments
        if p_memsz == 0 {
            continue;
        }

        // Validate segment fits in guest memory
        let end = p_vaddr as usize + p_memsz;
        if end > mem_size {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz as u64,
            });
        }

        // Validate file data is available
        if p_filesz > 0 && p_offset + p_filesz > data.len() {
            return Err(LoadError::SegmentTruncated);
        }

        // Copy file data into guest memory
        if p_filesz > 0 {
            let range =
                ByteRange::new(GuestAddr::new(p_vaddr), p_filesz as u64).expect("valid range");
            memory
                .apply_commit(range, &data[p_offset..p_offset + p_filesz])
                .expect("segment fits in memory");
        }

        // Zero BSS (memsz > filesz)
        if p_memsz > p_filesz {
            let bss_start = p_vaddr + p_filesz as u64;
            let bss_size = p_memsz - p_filesz;
            let range =
                ByteRange::new(GuestAddr::new(bss_start), bss_size as u64).expect("valid range");
            memory
                .apply_commit(range, &vec![0u8; bss_size])
                .expect("BSS fits in memory");
        }

        if end > max_addr {
            max_addr = end;
        }
    }

    // PPC64 ELF ABI v1: e_entry points to a function descriptor in
    // the .opd section, not directly to code. The descriptor layout
    // is { u32 code_addr, u32 toc } (PS3 uses 32-bit effective
    // addresses despite the 64-bit ELF container).
    let entry_off = entry as usize;
    let mem_bytes = memory.as_bytes();
    if entry_off + 8 <= mem_bytes.len() {
        let code_addr = u32::from_be_bytes([
            mem_bytes[entry_off],
            mem_bytes[entry_off + 1],
            mem_bytes[entry_off + 2],
            mem_bytes[entry_off + 3],
        ]);
        let toc = u32::from_be_bytes([
            mem_bytes[entry_off + 4],
            mem_bytes[entry_off + 5],
            mem_bytes[entry_off + 6],
            mem_bytes[entry_off + 7],
        ]);
        state.pc = code_addr as u64;
        state.gpr[2] = toc as u64;
    } else {
        state.pc = entry;
    }

    Ok(LoadResult {
        entry,
        min_memory_size: max_addr,
    })
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PpuState;

    #[test]
    fn rejects_too_small() {
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        assert_eq!(
            load_ppu_elf(&[0; 10], &mut mem, &mut s),
            Err(LoadError::TooSmall)
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        let mut data = [0u8; 64];
        data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::BadMagic)
        );
    }

    #[test]
    fn rejects_32bit_elf() {
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        let mut data = [0u8; 64];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 1; // 32-bit
        data[5] = 2; // big-endian
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::Not64Bit)
        );
    }

    #[test]
    fn rejects_little_endian_elf() {
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        let mut data = [0u8; 64];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // 64-bit (valid)
        data[5] = 1; // little-endian (rejected)
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::NotBigEndian)
        );
    }

    /// Build a minimal big-endian ELF64 header describing `phnum`
    /// program headers immediately following the file header. The
    /// program-header table starts at offset 64 (right after the
    /// ELF header) and each entry is 56 bytes.
    fn mk_elf_header(phnum: u16) -> Vec<u8> {
        let mut data = vec![0u8; 64 + 56 * phnum as usize];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // 64-bit
        data[5] = 2; // big-endian
                     // e_phoff at offset 32: 8 bytes BE = 64
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        // e_phentsize at offset 54: 2 bytes BE = 56
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        // e_phnum at offset 56: 2 bytes BE
        data[56..58].copy_from_slice(&phnum.to_be_bytes());
        data
    }

    /// Patch a PT_LOAD program header at `data[64 + slot * 56..]`.
    fn write_ph(
        data: &mut [u8],
        slot: usize,
        p_offset: u64,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
    ) {
        let base = 64 + slot * 56;
        // p_type = PT_LOAD at offset 0 (4 bytes)
        data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        // p_offset at offset 8 (8 bytes)
        data[base + 8..base + 16].copy_from_slice(&p_offset.to_be_bytes());
        // p_vaddr at offset 16 (8 bytes)
        data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
        // p_filesz at offset 32 (8 bytes)
        data[base + 32..base + 40].copy_from_slice(&p_filesz.to_be_bytes());
        // p_memsz at offset 40 (8 bytes)
        data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
    }

    #[test]
    fn rejects_segment_out_of_range() {
        // A PT_LOAD segment whose vaddr + memsz exceeds guest memory.
        let mut data = mk_elf_header(1);
        // Segment at vaddr 0 with memsz 512, into a 256-byte memory.
        write_ph(&mut data, 0, 64 + 56, 0, 0, 512);
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::SegmentOutOfRange {
                vaddr: 0,
                memsz: 512,
            })
        );
    }

    #[test]
    fn rejects_segment_truncated() {
        // A PT_LOAD segment whose p_offset + p_filesz exceeds the
        // actual file size.
        let mut data = mk_elf_header(1);
        // Segment claims 100 bytes of file data starting at offset
        // 64 + 56 = 120, but the file is only 120 bytes long.
        write_ph(&mut data, 0, 120, 0, 100, 100);
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::SegmentTruncated)
        );
    }

    #[test]
    fn skips_empty_segment() {
        // A PT_LOAD with memsz == 0 must be skipped cleanly (not
        // faulted, not touching memory). A second empty segment
        // keeps the loop traversing program headers after the skip.
        let mut data = mk_elf_header(2);
        write_ph(&mut data, 0, 0, 0, 0, 0);
        write_ph(&mut data, 1, 0, 0, 0, 0);
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(256);
        let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load ok");
        assert_eq!(result.min_memory_size, 0);
    }

    #[test]
    fn loads_real_ppu_elf() {
        let path =
            std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
        if !path.exists() {
            return; // skip if not built
        }
        let data = std::fs::read(path).unwrap();
        let mut s = PpuState::new();
        // PS3 PPU binaries need ~256MB+ for the 0x10000000 region
        let mut mem = GuestMemory::new(0x10020000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
        // ELF e_entry is the function descriptor at 0x10000000, but
        // the loader resolves it to the actual code address.
        assert_eq!(result.entry, 0x10000000);
        // PSL1GHT descriptor points to code at 0x10200 in .text.
        assert_eq!(s.pc, 0x10200);
        // First instruction at the code address should be real code.
        let pc = s.pc as usize;
        let first_insn = u32::from_be_bytes([
            mem.as_bytes()[pc],
            mem.as_bytes()[pc + 1],
            mem.as_bytes()[pc + 2],
            mem.as_bytes()[pc + 3],
        ]);
        assert_ne!(first_insn, 0, "entry point should have code");
    }

    #[test]
    fn loads_rpcs3_test_binary() {
        let path = std::path::Path::new("../../tools/rpcs3/test/ppu_thread.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let mut s = PpuState::new();
        // SDK-compiled binaries use low addresses (~215KB)
        let mut mem = GuestMemory::new(0x40000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
        assert_eq!(result.entry, 0x301c0);
        // SDK descriptor at 0x301c0 points to code at 0x1022c,
        // TOC at 0x38b50.
        assert_eq!(s.pc, 0x1022c);
        assert_eq!(s.gpr[2], 0x38b50);
        let pc = s.pc as usize;
        let first_insn = u32::from_be_bytes([
            mem.as_bytes()[pc],
            mem.as_bytes()[pc + 1],
            mem.as_bytes()[pc + 2],
            mem.as_bytes()[pc + 3],
        ]);
        assert_ne!(first_insn, 0, "entry point should have code");
    }
}
