//! PPU ELF64 loader: copies PT_LOAD segments into guest memory and
//! resolves the entry-point OPD into `(pc, toc)`.

use crate::state::PpuState;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

/// Why loading failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// File is too small to contain an ELF header (or program-header
    /// table arithmetic overflowed -- treated identically: there is
    /// no usable ELF structure to load).
    TooSmall,
    /// ELF magic bytes (0x7F 'E' 'L' 'F') not found.
    BadMagic,
    /// Not a 64-bit ELF (PPU ELFs must be ELF64).
    Not64Bit,
    /// Not big-endian (PPU ELFs must be MSB).
    NotBigEndian,
    /// A LOAD segment extends past the end of the file.
    SegmentTruncated,
    /// A LOAD segment's virtual address + size exceeds guest memory,
    /// overflows a 32-bit PS3 effective address, or arithmetic on the
    /// vaddr/memsz pair overflowed.
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

/// Compute `phoff + i * phentsize` defensively. Returns `None` on overflow
/// or if the resulting program-header slot would extend past `data.len()`,
/// which the callers translate into `LoadError::TooSmall`. Attacker-controlled
/// header fields can otherwise wrap and bypass the post-bounds check.
fn ph_slot_base(data_len: usize, phoff: usize, phentsize: usize, i: usize) -> Option<usize> {
    let prod = i.checked_mul(phentsize)?;
    let base = phoff.checked_add(prod)?;
    let end = base.checked_add(phentsize)?;
    if end > data_len {
        return None;
    }
    Some(base)
}

/// Entry point and the minimum guest memory size needed to hold every
/// PT_LOAD segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadResult {
    /// ELF entry point (set as state.pc).
    pub entry: u64,
    /// Minimum guest memory size to hold all segments.
    pub min_memory_size: usize,
}

/// Minimum guest memory needed to host every PT_LOAD (including BSS).
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

    let mut max_addr: u64 = 0;
    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i).ok_or(LoadError::TooSmall)?;
        let p_type = read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }
        let p_vaddr = read_u64(data, base + 16);
        let p_memsz = read_u64(data, base + 40);
        if p_memsz == 0 {
            continue;
        }
        let end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            })?;
        if end > u64::from(u32::MAX) + 1 {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            });
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

    let entry = read_u64(data, 24);
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    let mem_size = memory.as_bytes().len();
    let mut max_addr: u64 = 0;

    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i).ok_or(LoadError::TooSmall)?;

        let p_type = read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }

        let p_offset = read_u64(data, base + 8) as usize;
        let p_vaddr = read_u64(data, base + 16);
        let p_filesz = read_u64(data, base + 32);
        let p_memsz = read_u64(data, base + 40);

        if p_memsz == 0 {
            continue;
        }

        // PS3 effective addresses are 32-bit; reject anything that would
        // wrap on add or land above the 4 GiB EA ceiling before we cast
        // to usize for the apply_commit call below.
        let end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            })?;
        if end > u64::from(u32::MAX) + 1 || end > mem_size as u64 {
            return Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            });
        }

        if p_filesz > 0
            && p_offset
                .checked_add(p_filesz as usize)
                .is_none_or(|e| e > data.len())
        {
            return Err(LoadError::SegmentTruncated);
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
    }

    // PPC64 ELF ABI v1: e_entry names a descriptor { u32 code, u32 toc }
    // in .opd. PS3 effective addresses are 32-bit despite the 64-bit
    // container.
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
        min_memory_size: max_addr as usize,
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

/// Enumerate PT_LOAD segments in program-header order (zero-sized
/// segments omitted). Matches the order `load_ppu_elf` copies them.
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
        let base = ph_slot_base(data.len(), phoff, phentsize, i).ok_or(LoadError::TooSmall)?;
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

/// Full PT_TLS layout for per-thread TLS block reconstruction.
#[derive(Debug, Clone, Copy)]
pub struct TlsProgramHeader {
    /// Offset into the ELF file where the initialized bytes start.
    pub file_offset: u64,
    /// Virtual address of the primary thread's TLS block.
    pub vaddr: u64,
    /// Count of initialized bytes (the `.tdata` payload length).
    pub filesz: u64,
    /// Total per-thread size (filesz plus `.tbss` zero-init tail).
    pub memsz: u64,
    /// Required alignment for per-thread TLS blocks.
    pub align: u64,
}

/// PT_TLS segment info, or `None` if the ELF has no TLS segment.
pub fn find_tls_segment(data: &[u8]) -> Option<TlsInfo> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i)?;
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

/// PT_TLS program header (including `p_offset` and `p_align`), or
/// `None` if absent.
pub fn find_tls_program_header(data: &[u8]) -> Option<TlsProgramHeader> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i)?;
        if read_u32(data, base) == PT_TLS {
            return Some(TlsProgramHeader {
                file_offset: read_u64(data, base + 8),
                vaddr: read_u64(data, base + 16),
                filesz: read_u64(data, base + 32),
                memsz: read_u64(data, base + 40),
                align: read_u64(data, base + 48),
            });
        }
    }
    None
}

/// PT_TLS initialized bytes plus `(memsz, align, vaddr)`.
///
/// `initial_bytes.len() == filesz` on success.
pub fn extract_tls_template_bytes(data: &[u8]) -> Option<(Vec<u8>, u64, u64, u64)> {
    let hdr = find_tls_program_header(data)?;
    let start = hdr.file_offset as usize;
    let end = start.checked_add(hdr.filesz as usize)?;
    if end > data.len() {
        return None;
    }
    Some((data[start..end].to_vec(), hdr.memsz, hdr.align, hdr.vaddr))
}

/// Magic marking the `.sys_proc_param` section.
const SYS_PROCESS_PARAM_MAGIC: u32 = 0x13bcc5f6;

/// Parsed `sys_process_param_t`. The caller passes `malloc_pagesize`
/// into the game entry via `r12` so the CRT0 sizes its allocator.
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

/// Whether file offset `file_off` falls within any PT_LOAD's
/// `[p_offset, p_offset + p_filesz)` file range. Used to filter
/// magic-scan false positives in string tables, debug sections, or
/// embedded asset data.
fn file_offset_in_pt_load(data: &[u8], file_off: usize) -> bool {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return false;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;
    for i in 0..phnum {
        let Some(base) = ph_slot_base(data.len(), phoff, phentsize, i) else {
            return false;
        };
        if read_u32(data, base) != PT_LOAD {
            continue;
        }
        let p_offset = read_u64(data, base + 8) as usize;
        let p_filesz = read_u64(data, base + 32) as usize;
        let Some(p_end) = p_offset.checked_add(p_filesz) else {
            continue;
        };
        if file_off >= p_offset && file_off < p_end {
            return true;
        }
    }
    false
}

/// Locate `.sys_proc_param` by scanning for its magic (avoids parsing
/// section headers). `None` if the magic is absent. Matches outside any
/// PT_LOAD file range are rejected so a stray byte sequence in a string
/// table, debug section, or embedded asset cannot masquerade as a real
/// `sys_process_param_t`.
pub fn find_sys_process_param(data: &[u8]) -> Option<SysProcessParam> {
    let magic_bytes = SYS_PROCESS_PARAM_MAGIC.to_be_bytes();
    // Struct: { u32 size, magic, version, sdk_version, i32 primary_prio,
    //           u32 primary_stacksize, malloc_pagesize, ppc_seg }. Magic
    // is at offset 4 of the struct.
    let mut idx = 0;
    while idx + 4 <= data.len() {
        let rel = data[idx..].windows(4).position(|w| w == magic_bytes)?;
        let s = idx + rel;
        if s < 4 || s + 28 > data.len() {
            idx = s + 4;
            continue;
        }
        let start = s - 4;
        let size = read_u32(data, start);
        if size < 0x20 {
            idx = s + 4;
            continue;
        }
        if !file_offset_in_pt_load(data, start) {
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
    }
    None
}

/// ELF section type: symbol table.
const SHT_SYMTAB: u32 = 2;
/// ELF section type: dynamic-link symbol table. PRX-linked PS3 binaries
/// keep their exported symbols in `.dynsym`, not `.symtab`.
const SHT_DYNSYM: u32 = 11;

/// Symbol address by name, or `None` if not found or the ELF has no
/// symbol table. Searches every `SHT_SYMTAB` and `SHT_DYNSYM` section
/// in order.
pub fn find_symbol(data: &[u8], name: &str) -> Option<u64> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC {
        return None;
    }
    let shoff = read_u64(data, 40) as usize;
    let shentsize = read_u16(data, 58) as usize;
    let shnum = read_u16(data, 60) as usize;

    for i in 0..shnum {
        let sh = shoff.checked_add(i.checked_mul(shentsize)?)?;
        if sh.checked_add(shentsize)? > data.len() {
            return None;
        }
        let sh_type = read_u32(data, sh + 4);
        if sh_type != SHT_SYMTAB && sh_type != SHT_DYNSYM {
            continue;
        }
        let sym_off = read_u64(data, sh + 24) as usize;
        let sym_size = read_u64(data, sh + 32) as usize;
        let sym_entsize = read_u64(data, sh + 56) as usize;
        let strtab_idx = read_u32(data, sh + 40) as usize;

        let Some(str_sh) = shoff.checked_add(strtab_idx.checked_mul(shentsize)?) else {
            continue;
        };
        if str_sh.checked_add(shentsize)? > data.len() {
            continue;
        }
        let str_off = read_u64(data, str_sh + 24) as usize;
        let str_size = read_u64(data, str_sh + 32) as usize;
        let Some(str_end) = str_off.checked_add(str_size) else {
            continue;
        };
        if str_end > data.len() {
            continue;
        }
        let strtab = &data[str_off..str_end];

        if sym_entsize == 0 {
            continue;
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

    fn mk_elf_header(phnum: u16) -> Vec<u8> {
        let mut data = vec![0u8; 64 + 56 * phnum as usize];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&phnum.to_be_bytes());
        data
    }

    fn write_ph(
        data: &mut [u8],
        slot: usize,
        p_offset: u64,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
    ) {
        let base = 64 + slot * 56;
        data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[base + 8..base + 16].copy_from_slice(&p_offset.to_be_bytes());
        data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
        data[base + 32..base + 40].copy_from_slice(&p_filesz.to_be_bytes());
        data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
    }

    #[test]
    fn rejects_segment_out_of_range() {
        let mut data = mk_elf_header(1);
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
        let mut data = mk_elf_header(1);
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
        let mut mem = GuestMemory::new(0x10020000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
        assert_eq!(result.entry, 0x10000000);
        assert_eq!(s.pc, 0x10200);
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
        let mut mem = GuestMemory::new(0x40000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).unwrap();
        assert_eq!(result.entry, 0x301c0);
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
        // `result` is declared with __attribute__((aligned(128))).
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

    fn make_elf_with_tls(tls_vaddr: u64, tls_filesz: u64, tls_memsz: u64) -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 2;
        buf[5] = 2;
        buf[32..40].copy_from_slice(&64u64.to_be_bytes());
        buf[54..56].copy_from_slice(&56u16.to_be_bytes());
        buf[56..58].copy_from_slice(&1u16.to_be_bytes());
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

    fn make_elf_with_tls_payload(
        tls_vaddr: u64,
        initial: &[u8],
        tls_memsz: u64,
        tls_align: u64,
    ) -> Vec<u8> {
        let payload_offset = 128u64;
        let total = (payload_offset as usize) + initial.len() + 16;
        let mut buf = vec![0u8; total];
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 2;
        buf[5] = 2;
        buf[32..40].copy_from_slice(&64u64.to_be_bytes());
        buf[54..56].copy_from_slice(&56u16.to_be_bytes());
        buf[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        buf[ph..ph + 4].copy_from_slice(&PT_TLS.to_be_bytes());
        buf[ph + 8..ph + 16].copy_from_slice(&payload_offset.to_be_bytes());
        buf[ph + 16..ph + 24].copy_from_slice(&tls_vaddr.to_be_bytes());
        buf[ph + 32..ph + 40].copy_from_slice(&(initial.len() as u64).to_be_bytes());
        buf[ph + 40..ph + 48].copy_from_slice(&tls_memsz.to_be_bytes());
        buf[ph + 48..ph + 56].copy_from_slice(&tls_align.to_be_bytes());
        let start = payload_offset as usize;
        buf[start..start + initial.len()].copy_from_slice(initial);
        buf
    }

    #[test]
    fn find_tls_program_header_returns_all_fields() {
        let initial = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let data = make_elf_with_tls_payload(0x895cd0, &initial, 0x1dc, 0x10);
        let hdr = find_tls_program_header(&data).expect("should find PT_TLS");
        assert_eq!(hdr.file_offset, 128);
        assert_eq!(hdr.vaddr, 0x895cd0);
        assert_eq!(hdr.filesz, 6);
        assert_eq!(hdr.memsz, 0x1dc);
        assert_eq!(hdr.align, 0x10);
    }

    #[test]
    fn extract_tls_template_bytes_captures_initial_payload() {
        let initial = [0x11u8, 0x22, 0x33, 0x44, 0x55];
        let data = make_elf_with_tls_payload(0x10_0000, &initial, 0x100, 0x20);
        let (bytes, memsz, align, vaddr) =
            extract_tls_template_bytes(&data).expect("should extract PT_TLS bytes");
        assert_eq!(bytes, initial);
        assert_eq!(memsz, 0x100);
        assert_eq!(align, 0x20);
        assert_eq!(vaddr, 0x10_0000);
    }

    #[test]
    fn extract_tls_template_bytes_returns_none_when_no_tls() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        data[64..68].copy_from_slice(&PT_LOAD.to_be_bytes());
        assert!(extract_tls_template_bytes(&data).is_none());
    }

    #[test]
    fn find_tls_returns_none_without_tls() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
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
        let mut data = mk_elf_header(2);
        write_ph(&mut data, 0, 0x100, 0x1000, 0x40, 0x40);
        // PF_R | PF_X
        data[64 + 4..64 + 8].copy_from_slice(&0x5u32.to_be_bytes());
        write_ph(&mut data, 1, 0x200, 0x2000, 0x20, 0x80);
        // PF_R | PF_W
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
    fn rejects_segment_with_vaddr_memsz_overflow() {
        // Crafted PT_LOAD: p_vaddr near u64::MAX so p_vaddr + p_memsz wraps.
        // Pre-fix arithmetic would wrap, pass the post-bounds check, and
        // route an OOB write through apply_commit.
        let mut data = mk_elf_header(1);
        let p_vaddr = u64::MAX - 0xFF;
        let p_memsz = 0x200u64;
        let base = 64;
        data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
        data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(0x10000);
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            })
        );
    }

    #[test]
    fn rejects_segment_above_ps3_ea_ceiling() {
        // PS3 effective addresses are 32-bit. A segment ending above 4 GiB
        // must be rejected even on 64-bit hosts where the arithmetic does
        // not overflow, so guest memory is never sized for an EA the
        // architecture cannot represent.
        let mut data = mk_elf_header(1);
        let p_vaddr = 0x1_0000_0000u64;
        let p_memsz = 0x10u64;
        let base = 64;
        data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[base + 16..base + 24].copy_from_slice(&p_vaddr.to_be_bytes());
        data[base + 40..base + 48].copy_from_slice(&p_memsz.to_be_bytes());
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(0x10000);
        assert_eq!(
            load_ppu_elf(&data, &mut mem, &mut s),
            Err(LoadError::SegmentOutOfRange {
                vaddr: p_vaddr,
                memsz: p_memsz,
            })
        );
    }

    #[test]
    fn find_tls_rejects_non_64bit_elf() {
        // A 32-bit ELF has different program-header field offsets. Without
        // the arch check, find_tls_segment would parse u64 fields against
        // the wrong layout and return garbage.
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 1; // 32-bit
        data[5] = 2; // big-endian
        assert!(find_tls_segment(&data).is_none());
        assert!(find_tls_program_header(&data).is_none());
    }

    #[test]
    fn find_tls_rejects_little_endian_elf() {
        let mut data = vec![0u8; 256];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // 64-bit
        data[5] = 1; // little-endian
        assert!(find_tls_segment(&data).is_none());
        assert!(find_tls_program_header(&data).is_none());
    }

    #[test]
    fn find_sys_process_param_rejects_magic_outside_pt_load() {
        // Build an ELF whose PT_LOAD covers a small file range that does
        // NOT contain the magic, and place the magic + plausible struct
        // bytes outside that range. Pre-fix scan would match the magic and
        // return false-positive struct fields.
        let payload_offset = 0x200usize;
        let pt_load_offset = 0x100usize;
        let pt_load_size = 0x40usize; // does not cover payload_offset
        let mut data = vec![0u8; payload_offset + 32 + 32];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
        data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        // Plant a plausible sys_proc_param: { size=0x30, magic, ... } at payload_offset.
        let start = payload_offset;
        data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
        data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
        assert!(
            find_sys_process_param(&data).is_none(),
            "magic outside PT_LOAD must be rejected as a false positive"
        );
    }

    #[test]
    fn find_sys_process_param_accepts_magic_inside_pt_load() {
        // Symmetric counterpart: a magic match inside the PT_LOAD file
        // range still parses as a real struct. Locks the positive path
        // so the PT_LOAD filter does not over-reject valid binaries.
        let payload_offset = 0x140usize;
        let pt_load_offset = 0x100usize;
        let pt_load_size = 0x80usize; // covers payload_offset
        let mut data = vec![0u8; pt_load_offset + pt_load_size + 32];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
        data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        let start = payload_offset;
        data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
        data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
        // sdk_version at start+12, primary_prio at +16, primary_stacksize at +20,
        // malloc_pagesize at +24, ppc_seg at +28
        data[start + 12..start + 16].copy_from_slice(&0x0015_0004u32.to_be_bytes());
        data[start + 16..start + 20].copy_from_slice(&1000i32.to_be_bytes());
        data[start + 20..start + 24].copy_from_slice(&0x10000u32.to_be_bytes());
        data[start + 24..start + 28].copy_from_slice(&0x10000u32.to_be_bytes());
        let p = find_sys_process_param(&data).expect("magic inside PT_LOAD must parse");
        assert_eq!(p.sdk_version, 0x0015_0004);
        assert_eq!(p.primary_prio, 1000);
        assert_eq!(p.primary_stacksize, 0x10000);
        assert_eq!(p.malloc_pagesize, 0x10000);
    }

    #[test]
    fn find_tls_on_real_elf() {
        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let tls =
            find_tls_segment(&data).expect("retail EBOOT should have a PT_TLS program header");
        assert_eq!(tls.vaddr, 0x895cd0);
        assert_eq!(tls.filesz, 4);
        assert_eq!(tls.memsz, 0x1dc);
    }
}
