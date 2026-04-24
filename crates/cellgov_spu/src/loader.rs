//! SPU ELF loader covering the subset used by PSL1GHT-compiled SPU
//! binaries (ELF32, big-endian, PT_LOAD segments only).

use crate::state::SpuState;

/// Load failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// File is too small to contain an ELF header.
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    BadMagic,
    /// Not a 32-bit ELF.
    Not32Bit,
    /// Not big-endian.
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

const ELF_HEADER_SIZE: usize = 52;
const PHDR_SIZE: usize = 32;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;

/// Load an SPU ELF binary into `state`, copying PT_LOAD segments into
/// LS, zeroing `.bss` (memsz > filesz), and setting `state.pc` to the
/// ELF entry point.
///
/// # Errors
///
/// Returns [`LoadError`] on any header or segment validation failure.
pub fn load_spu_elf(data: &[u8], state: &mut SpuState) -> Result<(), LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }

    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }

    // EI_CLASS must be 1 (32-bit).
    if data[4] != 1 {
        return Err(LoadError::Not32Bit);
    }

    // EI_DATA must be 2 (big-endian).
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }

    let entry = read_u32(data, 24);
    let phoff = read_u32(data, 28) as usize;
    let phnum = read_u16(data, 44) as usize;
    let phentsize = read_u16(data, 42) as usize;

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
            return;
        }
        let data = std::fs::read(path).unwrap();
        let mut s = SpuState::new();
        load_spu_elf(&data, &mut s).unwrap();
        assert_eq!(s.pc, 0x160);
        let first_insn = u32::from_be_bytes([s.ls[0], s.ls[1], s.ls[2], s.ls[3]]);
        assert_eq!(first_insn, 0x7b00_0000);
    }
}
