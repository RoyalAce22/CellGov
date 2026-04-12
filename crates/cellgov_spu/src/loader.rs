//! Minimal SPU ELF loader.
//!
//! Reads an ELF32 binary (SPU ELFs are always 32-bit big-endian),
//! extracts LOAD program headers, and copies each segment into the
//! SPU's local store at the specified virtual address. Sets the
//! program counter to the ELF entry point.
//!
//! This is a test loader, not a production ELF loader. It handles
//! the subset of ELF features that PSL1GHT-compiled SPU binaries use.

use crate::state::SpuState;

/// Why loading failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// File is too small to contain an ELF header.
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    BadMagic,
    /// Not a 32-bit ELF (SPU ELFs must be ELF32).
    Not32Bit,
    /// Not big-endian (SPU ELFs must be MSB).
    NotBigEndian,
    /// A LOAD segment extends past the end of the file.
    SegmentTruncated,
    /// A LOAD segment's virtual address + size exceeds local store.
    SegmentOutOfRange {
        /// Virtual address of the segment.
        vaddr: u32,
        /// Memory size of the segment.
        memsz: u32,
    },
}

/// ELF32 header size.
const ELF_HEADER_SIZE: usize = 52;
/// ELF32 program header entry size.
const PHDR_SIZE: usize = 32;
/// ELF magic.
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
/// PT_LOAD segment type.
const PT_LOAD: u32 = 1;

/// Load an SPU ELF binary into the given SPU state.
///
/// Copies all PT_LOAD segments into local store and sets `state.pc`
/// to the ELF entry point. The `.bss` portion (memsz > filesz) is
/// zeroed.
pub fn load_spu_elf(data: &[u8], state: &mut SpuState) -> Result<(), LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }

    // Validate ELF magic
    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }

    // EI_CLASS must be 1 (32-bit)
    if data[4] != 1 {
        return Err(LoadError::Not32Bit);
    }

    // EI_DATA must be 2 (big-endian)
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }

    // Entry point: offset 24, 4 bytes BE
    let entry = read_u32(data, 24);

    // Program header table offset: offset 28, 4 bytes BE
    let phoff = read_u32(data, 28) as usize;

    // Number of program headers: offset 44, 2 bytes BE
    let phnum = read_u16(data, 44) as usize;

    // Program header entry size: offset 42, 2 bytes BE
    let phentsize = read_u16(data, 42) as usize;

    // Process each program header
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + PHDR_SIZE > data.len() {
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

        // Validate segment fits in LS
        let end = p_vaddr as usize + p_memsz;
        if end > state.ls.len() {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz as u32,
            });
        }

        // Validate file data is available
        if p_offset + p_filesz > data.len() {
            return Err(LoadError::SegmentTruncated);
        }

        // Copy file data into LS
        let dst_start = p_vaddr as usize;
        state.ls[dst_start..dst_start + p_filesz]
            .copy_from_slice(&data[p_offset..p_offset + p_filesz]);

        // Zero BSS (memsz > filesz)
        if p_memsz > p_filesz {
            let bss_start = dst_start + p_filesz;
            let bss_end = dst_start + p_memsz;
            state.ls[bss_start..bss_end].fill(0);
        }
    }

    state.pc = entry;
    Ok(())
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
    use crate::state::SpuState;

    #[test]
    fn rejects_too_small() {
        let mut s = SpuState::new();
        assert_eq!(load_spu_elf(&[0; 10], &mut s), Err(LoadError::TooSmall));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut s = SpuState::new();
        let mut data = [0u8; 52];
        data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(load_spu_elf(&data, &mut s), Err(LoadError::BadMagic));
    }

    #[test]
    fn rejects_64bit_elf() {
        let mut s = SpuState::new();
        let mut data = [0u8; 52];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // 64-bit
        data[5] = 2; // big-endian
        assert_eq!(load_spu_elf(&data, &mut s), Err(LoadError::Not32Bit));
    }

    #[test]
    fn loads_real_spu_elf() {
        let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
        if !path.exists() {
            return; // skip if not built
        }
        let data = std::fs::read(path).unwrap();
        let mut s = SpuState::new();
        load_spu_elf(&data, &mut s).unwrap();
        // Entry point should be 0x160 (from readelf output)
        assert_eq!(s.pc, 0x160);
        // Code segment starts at 0x000, first instruction at offset 0
        // should be `heq $0,$0,$0` = 0x7b000000
        let first_insn = u32::from_be_bytes([s.ls[0], s.ls[1], s.ls[2], s.ls[3]]);
        assert_eq!(first_insn, 0x7b00_0000);
    }
}
