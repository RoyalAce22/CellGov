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

/// Scan program headers and return the minimum guest memory size
/// needed to hold all PT_LOAD segments (including BSS). Does not
/// modify any state.
pub fn required_memory_size(data: &[u8]) -> Result<usize, LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }
    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }
    if data[4] != 2 {
        return Err(LoadError::Not64Bit);
    }
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }

    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    let mut max_addr: usize = 0;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            return Err(LoadError::TooSmall);
        }
        let p_type = read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }
        let p_vaddr = read_u64(data, base + 16);
        let p_memsz = read_u64(data, base + 40) as usize;
        if p_memsz == 0 {
            continue;
        }
        let end = p_vaddr as usize + p_memsz;
        if end > max_addr {
            max_addr = end;
        }
    }
    Ok(max_addr)
}

/// Load a PPU ELF64 binary into guest memory and set the PC.
///
/// Copies all PT_LOAD segments with nonzero memsz into guest memory
/// at the specified virtual addresses. The `.bss` portion
/// (memsz > filesz) is zeroed. Resolves the ELF entry point through
/// its OPD: sets `state.pc` to the code address read from the OPD
/// and `state.gpr[2]` to the accompanying TOC pointer.
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

pub(crate) fn read_u64(data: &[u8], offset: usize) -> u64 {
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

pub(crate) fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

pub(crate) fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

/// A PT_LOAD segment's address range and permission bits.
///
/// Used to derive memory-region descriptors for cross-runner comparison:
/// read-only segments (code / rodata) must be byte-identical between
/// runners at any checkpoint; writable segments (data / bss) depend on
/// boot state and compare meaningfully only at matched checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    /// Index of the program header within the ELF.
    pub index: usize,
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

/// Enumerate every PT_LOAD segment in a PPU ELF64 binary.
///
/// Skips zero-sized segments. The returned order matches program header
/// order, which is the order `load_ppu_elf` copies them into guest
/// memory. Readers can classify a segment as code/rodata (not writable)
/// versus data/bss (writable) via the permission bits.
pub fn pt_load_segments(data: &[u8]) -> Result<Vec<LoadSegment>, LoadError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall);
    }
    if data[0..4] != ELF_MAGIC {
        return Err(LoadError::BadMagic);
    }
    if data[4] != 2 {
        return Err(LoadError::Not64Bit);
    }
    if data[5] != 2 {
        return Err(LoadError::NotBigEndian);
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;
    let mut out = Vec::new();
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            return Err(LoadError::TooSmall);
        }
        if read_u32(data, base) != PT_LOAD {
            continue;
        }
        let p_flags = read_u32(data, base + 4);
        let memsz = read_u64(data, base + 40);
        if memsz == 0 {
            continue;
        }
        out.push(LoadSegment {
            index: i,
            vaddr: read_u64(data, base + 16),
            filesz: read_u64(data, base + 32),
            memsz,
            executable: (p_flags & 0x1) != 0,
            writable: (p_flags & 0x2) != 0,
            readable: (p_flags & 0x4) != 0,
        });
    }
    Ok(out)
}

/// PT_TLS segment type.
const PT_TLS: u32 = 7;

/// TLS segment info extracted from an ELF's PT_TLS program header.
#[derive(Debug, Clone, Copy)]
pub struct TlsInfo {
    /// Virtual address of the TLS template in guest memory.
    pub vaddr: u64,
    /// Size of initialized TLS data (from ELF file).
    pub filesz: u64,
    /// Total TLS memory size per thread (including BSS).
    pub memsz: u64,
}

/// Find the PT_TLS program header in an ELF64 binary.
///
/// Returns `None` if the ELF has no TLS segment.
pub fn find_tls_segment(data: &[u8]) -> Option<TlsInfo> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            break;
        }
        if read_u32(data, base) == PT_TLS {
            return Some(TlsInfo {
                vaddr: read_u64(data, base + 16),
                filesz: read_u64(data, base + 32),
                memsz: read_u64(data, base + 40),
            });
        }
    }
    None
}

/// SYS_PROCESS_PARAM_MAGIC -- marks the .sys_proc_param section.
const SYS_PROCESS_PARAM_MAGIC: u32 = 0x13bcc5f6;

/// Parsed sys_process_param_t from the game ELF's .sys_proc_param section.
///
/// The PS3 kernel reads this section during process startup to configure
/// the primary PPU thread and the libc heap. CellGov mirrors RPCS3 in
/// passing `malloc_pagesize` into the game entry via r12 so the CRT0 can
/// initialize its allocator with the correct page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysProcessParam {
    /// SDK version that built the ELF (e.g., 0x150004 = SDK 1.5.0.4).
    pub sdk_version: u32,
    /// Primary PPU thread priority (0..3071).
    pub primary_prio: i32,
    /// Primary PPU thread stack size in bytes.
    pub primary_stacksize: u32,
    /// libc malloc page size (0x10000 = 64KB, 0x100000 = 1MB, 0 = unset).
    pub malloc_pagesize: u32,
    /// PPC segment mode (0 = default, 1 = OVLM).
    pub ppc_seg: u32,
}

/// Scan the raw ELF bytes for a `.sys_proc_param` section by magic.
///
/// The section is normally at a fixed offset in a PT_LOAD segment; locating
/// it by magic lets us find it without parsing section headers. Returns
/// `None` if the magic is not present.
pub fn find_sys_process_param(data: &[u8]) -> Option<SysProcessParam> {
    let magic_bytes = SYS_PROCESS_PARAM_MAGIC.to_be_bytes();
    // The struct layout is:
    //   u32 size, magic, version, sdk_version,
    //   i32 primary_prio,
    //   u32 primary_stacksize, malloc_pagesize, ppc_seg
    // Magic is at byte offset 4 of the struct.
    let mut idx = 0;
    while idx + 32 <= data.len() {
        if let Some(found) = data[idx..].windows(4).position(|w| w == magic_bytes) {
            let s = idx + found;
            if s < 4 || s + 28 > data.len() {
                return None;
            }
            let start = s - 4;
            let size = read_u32(data, start);
            if size < 0x20 {
                idx = s + 4;
                continue;
            }
            return Some(SysProcessParam {
                sdk_version: read_u32(data, start + 12),
                primary_prio: read_u32(data, start + 16) as i32,
                primary_stacksize: read_u32(data, start + 20),
                malloc_pagesize: read_u32(data, start + 24),
                ppc_seg: read_u32(data, start + 28),
            });
        } else {
            return None;
        }
    }
    None
}

/// ELF section type: symbol table.
const SHT_SYMTAB: u32 = 2;

/// Look up a symbol by name in an ELF64 big-endian binary and return
/// its value (address). Returns `None` if the ELF has no symbol table
/// or the symbol is not found.
pub fn find_symbol(data: &[u8], name: &str) -> Option<u64> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC {
        return None;
    }
    let shoff = read_u64(data, 40) as usize;
    let shentsize = read_u16(data, 58) as usize;
    let shnum = read_u16(data, 60) as usize;

    // Find the SHT_SYMTAB section.
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        if sh + shentsize > data.len() {
            return None;
        }
        let sh_type = read_u32(data, sh + 4);
        if sh_type != SHT_SYMTAB {
            continue;
        }
        let sym_off = read_u64(data, sh + 24) as usize;
        let sym_size = read_u64(data, sh + 32) as usize;
        let sym_entsize = read_u64(data, sh + 56) as usize;
        let strtab_idx = read_u32(data, sh + 40) as usize;

        // Read the associated string table section header.
        let str_sh = shoff + strtab_idx * shentsize;
        if str_sh + shentsize > data.len() {
            return None;
        }
        let str_off = read_u64(data, str_sh + 24) as usize;
        let str_size = read_u64(data, str_sh + 32) as usize;
        if str_off + str_size > data.len() {
            return None;
        }
        let strtab = &data[str_off..str_off + str_size];

        // Scan symbols.
        if sym_entsize == 0 {
            return None;
        }
        let count = sym_size / sym_entsize;
        for j in 0..count {
            let entry = sym_off + j * sym_entsize;
            if entry + sym_entsize > data.len() {
                break;
            }
            let st_name = read_u32(data, entry) as usize;
            if st_name >= strtab.len() {
                continue;
            }
            let end = strtab[st_name..]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(strtab.len() - st_name);
            let sym_name = &strtab[st_name..st_name + end];
            if sym_name == name.as_bytes() {
                return Some(read_u64(data, entry + 8));
            }
        }
        return None; // only one SYMTAB expected
    }
    None
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

    #[test]
    fn find_symbol_locates_result_in_ppu_elf() {
        let path =
            std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let addr = find_symbol(&data, "result");
        assert!(addr.is_some(), "symbol 'result' not found in ELF");
        // The address should be 128-byte aligned (the C code uses
        // __attribute__((aligned(128)))).
        assert_eq!(addr.unwrap() % 128, 0);
    }

    #[test]
    fn find_symbol_returns_none_for_missing() {
        let path =
            std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        assert!(find_symbol(&data, "nonexistent_symbol_xyz").is_none());
    }

    /// Build a minimal ELF64 with a PT_TLS program header for testing.
    fn make_elf_with_tls(tls_vaddr: u64, tls_filesz: u64, tls_memsz: u64) -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 2; // 64-bit
        buf[5] = 2; // big-endian
                    // phoff = 64, phentsize = 56, phnum = 1
        buf[32..40].copy_from_slice(&64u64.to_be_bytes());
        buf[54..56].copy_from_slice(&56u16.to_be_bytes());
        buf[56..58].copy_from_slice(&1u16.to_be_bytes());
        // PT_TLS at phdr[0]
        let ph = 64;
        buf[ph..ph + 4].copy_from_slice(&PT_TLS.to_be_bytes());
        buf[ph + 16..ph + 24].copy_from_slice(&tls_vaddr.to_be_bytes());
        buf[ph + 32..ph + 40].copy_from_slice(&tls_filesz.to_be_bytes());
        buf[ph + 40..ph + 48].copy_from_slice(&tls_memsz.to_be_bytes());
        buf
    }

    #[test]
    fn find_tls_returns_correct_info() {
        let data = make_elf_with_tls(0x895cd0, 4, 0x1dc);
        let tls = find_tls_segment(&data).expect("should find PT_TLS");
        assert_eq!(tls.vaddr, 0x895cd0);
        assert_eq!(tls.filesz, 4);
        assert_eq!(tls.memsz, 0x1dc);
    }

    #[test]
    fn find_tls_returns_none_without_tls() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        // PT_LOAD, not PT_TLS
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        data[64..68].copy_from_slice(&PT_LOAD.to_be_bytes());
        assert!(find_tls_segment(&data).is_none());
    }

    #[test]
    fn find_tls_returns_none_for_bad_magic() {
        let data = vec![0u8; 128];
        assert!(find_tls_segment(&data).is_none());
    }

    #[test]
    fn find_tls_returns_none_for_short_input() {
        assert!(find_tls_segment(&[0; 10]).is_none());
    }

    #[test]
    fn pt_load_segments_enumerates_all() {
        // Two PT_LOAD segments with different permission bits.
        let mut data = mk_elf_header(2);
        write_ph(&mut data, 0, 0x100, 0x1000, 0x40, 0x40);
        // Set PF_R|PF_X (0x5) on segment 0.
        data[64 + 4..64 + 8].copy_from_slice(&0x5u32.to_be_bytes());
        write_ph(&mut data, 1, 0x200, 0x2000, 0x20, 0x80);
        // Set PF_R|PF_W (0x6) on segment 1.
        data[64 + 56 + 4..64 + 56 + 8].copy_from_slice(&0x6u32.to_be_bytes());
        let segs = pt_load_segments(&data).expect("parses");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].vaddr, 0x1000);
        assert_eq!(segs[0].memsz, 0x40);
        assert!(segs[0].executable);
        assert!(!segs[0].writable);
        assert!(segs[0].readable);
        assert_eq!(segs[1].vaddr, 0x2000);
        assert_eq!(segs[1].filesz, 0x20);
        assert_eq!(segs[1].memsz, 0x80);
        assert!(!segs[1].executable);
        assert!(segs[1].writable);
        assert!(segs[1].readable);
    }

    #[test]
    fn pt_load_segments_skips_zero_memsz() {
        let mut data = mk_elf_header(2);
        write_ph(&mut data, 0, 0x100, 0x1000, 0, 0);
        write_ph(&mut data, 1, 0x200, 0x2000, 0x10, 0x10);
        let segs = pt_load_segments(&data).expect("parses");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].vaddr, 0x2000);
    }

    #[test]
    fn find_tls_on_real_elf() {
        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let tls = find_tls_segment(&data).expect("flOw ELF should have PT_TLS");
        assert_eq!(tls.vaddr, 0x895cd0);
        assert_eq!(tls.filesz, 4);
        assert_eq!(tls.memsz, 0x1dc);
    }
}
