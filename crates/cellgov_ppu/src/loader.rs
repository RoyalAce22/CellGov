//! PPU ELF64 loader: copies PT_LOAD segments into guest memory and
//! resolves the entry-point OPD into `(pc, toc)`.

use crate::state::PpuState;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

/// `(addr, size)` pair describing where a segment would have been
/// placed. Shared by [`LoadError::SegmentOutOfRange`] (ELF PT_LOAD)
/// and [`crate::sprx::PrxLoadError::SegmentOutOfRange`] (PRX
/// text / data) so the two failure shapes diagnose identically.
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
    /// File is too small to contain an ELF header (or program-header
    /// table arithmetic overflowed -- treated identically: there is
    /// no usable ELF structure to load).
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
    /// A LOAD segment extends past the end of the file.
    #[error("PPU ELF LOAD segment truncated")]
    SegmentTruncated,
    /// A LOAD segment's virtual address + size exceeds guest memory,
    /// overflows a 32-bit PS3 effective address, or arithmetic on the
    /// vaddr/memsz pair overflowed. `segment_index` is the offending
    /// segment's slot in the program-header table.
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
}

use cellgov_ps3_abi::elf::{ELF_HEADER_SIZE, ELF_MAGIC, PT_LOAD};

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
        if end > u64::from(u32::MAX) + 1 {
            return Err(LoadError::SegmentOutOfRange {
                placement,
                segment_index: i,
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

    let sys_proc_param_range =
        find_sys_process_param(data).map(|p| p.guest_addr..p.guest_addr + p.struct_size as u64);

    Ok(LoadResult {
        entry,
        min_memory_size: max_addr as usize,
        sys_proc_param_range,
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

use cellgov_ps3_abi::elf::PT_TLS;

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

use cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_MAGIC;

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
    /// Guest address where the struct lives after the ELF is loaded,
    /// derived by mapping the struct's file offset through the
    /// containing PT_LOAD segment.
    pub guest_addr: u64,
    /// Value of the on-disk `size` field at struct offset 0
    /// (typically 0x30 or 0x40 across observed SDKs).
    pub struct_size: u32,
}

/// Locate the PT_LOAD whose file range covers `file_off` and return
/// the corresponding guest virtual address. Returns `None` if no
/// PT_LOAD covers the offset; used both to filter magic-scan false
/// positives and to compute the guest location of structs found by
/// scanning.
fn pt_load_file_to_guest(data: &[u8], file_off: usize) -> Option<u64> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;
    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i)?;
        if read_u32(data, base) != PT_LOAD {
            continue;
        }
        let p_offset = read_u64(data, base + 8) as usize;
        let p_vaddr = read_u64(data, base + 16);
        let p_filesz = read_u64(data, base + 32) as usize;
        let p_end = p_offset.checked_add(p_filesz)?;
        if file_off >= p_offset && file_off < p_end {
            return Some(p_vaddr + (file_off - p_offset) as u64);
        }
    }
    None
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
        let Some(guest_addr) = pt_load_file_to_guest(data, start) else {
            idx = s + 4;
            continue;
        };
        return Some(SysProcessParam {
            sdk_version: read_u32(data, start + 12),
            primary_prio: read_u32(data, start + 16) as i32,
            primary_stacksize: read_u32(data, start + 20),
            malloc_pagesize: read_u32(data, start + 24),
            ppc_seg: read_u32(data, start + 28),
            guest_addr,
            struct_size: size,
        });
    }
    None
}

/// Secondary OPD pointer table located by 8-byte header signature.
///
/// The PRX-link CRT0 walker patches these tables at runtime with
/// HLE OPD addresses from the same address space as the primary
/// import-stub table; the cross-runner classifier treats bytes
/// inside these tables under the same `HleOpdSlot` rule that covers
/// the primary table. Located by scan because the tables sit in the
/// title's `.data` section, outside the SCE PRX_PARAM-described
/// `lib_stub_start..lib_stub_end` primary import area.
///
/// Observed on SSHD (NPUA80068) and WipEout (BCES00664):
///
/// - 8-byte header: `04 02 NN 00  00 NN 00 00` where NN is a
///   sequence-number byte (`01` on the first table, `02` on the
///   second; identical across both titles).
/// - 0x60 bytes of slot data following the header.
/// - Two tables per title, adjacent in the data segment (second
///   table's `guest_addr` equals first's `guest_addr + 0x68`).
///
/// Writer attribution: SSHD's PC 0x4dec64 is an FNID-lookup loop
/// over `ppu_prx_module_info` nodes; the loop runs during CRT0 and
/// rewrites each slot's static self-referential `.data` trampoline
/// address with the loader's HLE OPD address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryOpdTable {
    /// Guest virtual address of the table's first byte (header).
    pub guest_addr: u64,
    /// Total table size in bytes (header + slot array).
    pub size: u64,
}

/// Total size of one secondary OPD table (header + slot array).
pub const SECONDARY_OPD_TABLE_SIZE: u64 = 0x68;

/// Locate every secondary OPD table in `data` by header-signature
/// scan over the EBOOT file. Candidates outside any PT_LOAD file
/// range are rejected via the same filter [`find_sys_process_param`]
/// uses, so stray byte sequences in section-header strings or
/// embedded assets cannot masquerade as real tables. Returns tables
/// in file-order; caller is responsible for merging adjacent extents
/// into a single classifier range if desired.
///
/// The scan is 4-byte aligned. A title whose PT_LOAD `p_offset` is
/// not 4-byte aligned would miss legitimate matches, but the
/// trade-off favours false-negatives over false-positives in stray
/// data; the four-byte stride is consistent with the PPC OPD-pointer
/// natural alignment.
pub fn find_secondary_opd_tables(data: &[u8]) -> Vec<SecondaryOpdTable> {
    let mut out = Vec::new();
    if data.len() < 8 {
        return out;
    }
    let mut i = 0usize;
    while i + 8 <= data.len() {
        let w0 = read_u32(data, i);
        let w1 = read_u32(data, i + 4);
        let w0_seq = (w0 >> 8) & 0xFF;
        let w1_seq = (w1 >> 16) & 0xFF;
        let header_match = (w0 & 0xFFFF_00FF) == 0x0402_0000
            && w0_seq != 0
            && (w1 & 0xFF00_FFFF) == 0
            && w1_seq != 0
            && w0_seq == w1_seq;
        if header_match {
            if let Some(guest_addr) = pt_load_file_to_guest(data, i) {
                out.push(SecondaryOpdTable {
                    guest_addr,
                    size: SECONDARY_OPD_TABLE_SIZE,
                });
                i += SECONDARY_OPD_TABLE_SIZE as usize;
                continue;
            }
        }
        i += 4;
    }
    out
}

/// One contiguous (id, ptr, opd_slot) triple-table observed in EBOOT
/// data. Field layout per row (4 bytes each):
///
/// - Bytes 0..4: caller-side identifier (counter, opcode, etc.).
/// - Bytes 4..8: pointer into the title's executable text segment.
/// - Bytes 8..12: OPD pointer slot. The CRT0 / PRX-link walker
///   rewrites this with an HLE OPD address at runtime; per-runner
///   addresses differ but the resolved function is equivalent.
///
/// Found by [`find_indirect_opd_tables`]: a run of consecutive
/// 12-byte rows where column 1 sits inside the executable PT_LOAD
/// range. WipEout's table at `data@0xc1110` is the post-Phase-38
/// driving observation; SSHD-shaped titles may have a sibling table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndirectOpdTable {
    /// Guest virtual address of the table's first row's first byte.
    pub guest_addr: u64,
    /// Total table size in bytes (= `count * INDIRECT_OPD_TABLE_STRIDE`).
    pub size: u64,
}

/// Per-row stride of an indirect OPD table.
pub const INDIRECT_OPD_TABLE_STRIDE: u64 = 12;

/// Byte offset of the OPD pointer slot within one row.
pub const INDIRECT_OPD_TABLE_SLOT_OFFSET: u64 = 8;

/// Minimum consecutive rows required to claim an indirect-OPD-table
/// extent. Four rows is enough to suppress two-pointer coincidences
/// (a 16-byte block of identical structure) while still catching small
/// tables. WipEout's table is ~60 rows; far above threshold.
const INDIRECT_OPD_TABLE_MIN_ROWS: usize = 4;

/// Locate every indirect-OPD table in `data` by scanning for runs of
/// 12-byte rows whose column-1 (bytes 4..8) is a pointer into the
/// title's executable PT_LOAD range. Each detected run with at least
/// [`INDIRECT_OPD_TABLE_MIN_ROWS`] rows is emitted as a single
/// [`IndirectOpdTable`]; callers derive the per-row OPD slot positions
/// at offset [`INDIRECT_OPD_TABLE_SLOT_OFFSET`].
///
/// The scan is 4-byte aligned and rejects matches outside any PT_LOAD
/// file range so stray byte sequences cannot masquerade as real tables.
/// False positives in non-table data are bounded by the row-count
/// threshold; false negatives on tables with fewer than four rows are
/// preferred to misclassifying random pointer pairs.
pub fn find_indirect_opd_tables(data: &[u8]) -> Vec<IndirectOpdTable> {
    let mut out = Vec::new();
    let Ok(segs) = pt_load_segments(data) else {
        return out;
    };
    let exec_ranges: Vec<std::ops::Range<u64>> = segs
        .iter()
        .filter(|s| s.executable)
        .map(|s| s.vaddr..s.vaddr.saturating_add(s.memsz))
        .collect();
    if exec_ranges.is_empty() {
        return out;
    }
    let is_code_ptr = |p: u32| -> bool {
        let p = u64::from(p);
        exec_ranges.iter().any(|r| r.contains(&p))
    };
    let stride = INDIRECT_OPD_TABLE_STRIDE as usize;

    let mut i = 0usize;
    while i + stride <= data.len() {
        let col1 = read_u32(data, i + 4);
        if !is_code_ptr(col1) {
            i += 4;
            continue;
        }
        let Some(start_addr) = pt_load_file_to_guest(data, i) else {
            i += 4;
            continue;
        };
        let mut rows = 1usize;
        let mut j = i + stride;
        while j + stride <= data.len() && is_code_ptr(read_u32(data, j + 4)) {
            rows += 1;
            j += stride;
        }
        if rows >= INDIRECT_OPD_TABLE_MIN_ROWS {
            out.push(IndirectOpdTable {
                guest_addr: start_addr,
                size: (rows as u64) * INDIRECT_OPD_TABLE_STRIDE,
            });
            i = j;
            continue;
        }
        i += stride;
    }
    out
}

use cellgov_ps3_abi::elf::{SHT_DYNSYM, SHT_SYMTAB};

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
                placement: SegmentPlacement { addr: 0, size: 512 },
                segment_index: 0,
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
        // Wrapping `p_vaddr + p_memsz` instead of using checked_add
        // can pass a post-bounds check on the wrapped value and
        // route an OOB write through apply_commit. Pins checked
        // arithmetic on segment-end computation.
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
                placement: SegmentPlacement {
                    addr: p_vaddr,
                    size: p_memsz,
                },
                segment_index: 0,
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
                placement: SegmentPlacement {
                    addr: p_vaddr,
                    size: p_memsz,
                },
                segment_index: 0,
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
        // bytes outside that range. A magic-scanner that walks raw
        // file bytes without filtering by PT_LOAD coverage would
        // match the magic and return false-positive struct fields.
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
        // guest_addr = p_vaddr (0) + (file_off 0x140 - p_offset 0x100) = 0x40.
        assert_eq!(p.guest_addr, 0x40);
        assert_eq!(p.struct_size, 0x30);
    }

    #[test]
    fn load_ppu_elf_populates_sys_proc_param_range_when_struct_present() {
        let payload_offset = 0x140usize;
        let pt_load_offset = 0x100usize;
        let pt_load_size = 0x80usize;
        let pt_load_vaddr: u64 = 0x10_0000;
        let mut data = vec![0u8; pt_load_offset + pt_load_size + 64];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph + 8..ph + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
        data[ph + 16..ph + 24].copy_from_slice(&pt_load_vaddr.to_be_bytes());
        data[ph + 32..ph + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        let start = payload_offset;
        data[start..start + 4].copy_from_slice(&0x30u32.to_be_bytes());
        data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());

        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(0x20_0000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load");
        // guest_addr = 0x10_0000 + (0x140 - 0x100) = 0x10_0040.
        // struct_size = 0x30.
        assert_eq!(
            result.sys_proc_param_range,
            Some(0x10_0040..0x10_0070),
            "sys_proc_param_range must cover [guest_addr, guest_addr + struct_size)",
        );
    }

    #[test]
    fn load_ppu_elf_leaves_sys_proc_param_range_none_without_struct() {
        let mut data = mk_elf_header(1);
        write_ph(&mut data, 0, 64 + 56, 0x10_0000, 0, 0);
        let mut s = PpuState::new();
        let mut mem = GuestMemory::new(0x20_0000);
        let result = load_ppu_elf(&data, &mut mem, &mut s).expect("load");
        assert!(result.sys_proc_param_range.is_none());
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

    /// Build an ELF with one PT_LOAD covering `[pt_off, pt_off+pt_sz)`
    /// at guest `pt_vaddr`, plus a writeable byte buffer the caller
    /// can plant table-header bytes into. Returns the assembled file.
    fn mk_elf_with_pt_load(pt_off: usize, pt_sz: usize, pt_vaddr: u64) -> Vec<u8> {
        let mut data = vec![0u8; pt_off + pt_sz + 16];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph + 8..ph + 16].copy_from_slice(&(pt_off as u64).to_be_bytes());
        data[ph + 16..ph + 24].copy_from_slice(&pt_vaddr.to_be_bytes());
        data[ph + 32..ph + 40].copy_from_slice(&(pt_sz as u64).to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&(pt_sz as u64).to_be_bytes());
        data
    }

    /// Write an 8-byte secondary-OPD-table header at `file_off`.
    fn plant_table_header(data: &mut [u8], file_off: usize, seq: u8) {
        data[file_off] = 0x04;
        data[file_off + 1] = 0x02;
        data[file_off + 2] = seq;
        data[file_off + 3] = 0x00;
        data[file_off + 4] = 0x00;
        data[file_off + 5] = seq;
        data[file_off + 6] = 0x00;
        data[file_off + 7] = 0x00;
    }

    #[test]
    fn find_secondary_opd_tables_finds_adjacent_pair() {
        let pt_off = 0x200usize;
        let pt_sz = 0x200usize;
        let pt_vaddr = 0x82_0000u64;
        let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);
        let t1_file = pt_off + 0x40;
        let t2_file = t1_file + 0x68;
        plant_table_header(&mut data, t1_file, 1);
        plant_table_header(&mut data, t2_file, 2);

        let tables = find_secondary_opd_tables(&data);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].guest_addr, pt_vaddr + 0x40);
        assert_eq!(tables[0].size, SECONDARY_OPD_TABLE_SIZE);
        assert_eq!(tables[1].guest_addr, pt_vaddr + 0x40 + 0x68);
        assert_eq!(tables[1].size, SECONDARY_OPD_TABLE_SIZE);
    }

    #[test]
    fn find_secondary_opd_tables_finds_single_when_only_one_present() {
        let pt_off = 0x200usize;
        let pt_sz = 0x100usize;
        let pt_vaddr = 0x82_0000u64;
        let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);
        let t1_file = pt_off + 0x40;
        plant_table_header(&mut data, t1_file, 1);

        let tables = find_secondary_opd_tables(&data);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].guest_addr, pt_vaddr + 0x40);
    }

    #[test]
    fn find_secondary_opd_tables_rejects_header_outside_pt_load() {
        // Plant the header at a file offset OUTSIDE any PT_LOAD range.
        // A scanner without the PT_LOAD filter would emit a phantom
        // table inside ELF padding / metadata.
        let pt_off = 0x200usize;
        let pt_sz = 0x40usize;
        let pt_vaddr = 0x82_0000u64;
        let outside_off = 0x300usize; // beyond pt_off + pt_sz = 0x240
        let mut data = vec![0u8; outside_off + 0x80];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        let ph = 64;
        data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph + 8..ph + 16].copy_from_slice(&(pt_off as u64).to_be_bytes());
        data[ph + 16..ph + 24].copy_from_slice(&pt_vaddr.to_be_bytes());
        data[ph + 32..ph + 40].copy_from_slice(&(pt_sz as u64).to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&(pt_sz as u64).to_be_bytes());
        plant_table_header(&mut data, outside_off, 1);

        let tables = find_secondary_opd_tables(&data);
        assert!(
            tables.is_empty(),
            "header outside PT_LOAD must be rejected; got {tables:?}"
        );
    }

    #[test]
    fn find_secondary_opd_tables_rejects_unaligned_or_mismatched_seq() {
        // Two adversarial cases:
        //   1. Header at a non-4-byte-aligned offset (scan strides
        //      by 4 from offset 0, so an unaligned plant is invisible).
        //   2. Header where the sequence byte in word 0 disagrees
        //      with the sequence byte in word 1 (e.g., `04 02 01 00
        //      00 02 00 00`). The match condition requires equality.
        let pt_off = 0x200usize;
        let pt_sz = 0x200usize;
        let pt_vaddr = 0x82_0000u64;
        let mut data = mk_elf_with_pt_load(pt_off, pt_sz, pt_vaddr);

        // Case 1: unaligned plant at +0x42 (not 4-byte aligned).
        let unaligned_off = pt_off + 0x42;
        plant_table_header(&mut data, unaligned_off, 1);
        let tables = find_secondary_opd_tables(&data);
        assert!(
            tables.is_empty(),
            "unaligned header must be missed by 4-byte-strided scan; got {tables:?}"
        );

        // Reset.
        for byte in &mut data[unaligned_off..unaligned_off + 8] {
            *byte = 0;
        }

        // Case 2: aligned plant with mismatched sequence bytes.
        let mismatch_off = pt_off + 0x40;
        data[mismatch_off] = 0x04;
        data[mismatch_off + 1] = 0x02;
        data[mismatch_off + 2] = 0x01;
        data[mismatch_off + 3] = 0x00;
        data[mismatch_off + 4] = 0x00;
        data[mismatch_off + 5] = 0x02; // != word-0 seq byte
        data[mismatch_off + 6] = 0x00;
        data[mismatch_off + 7] = 0x00;
        let tables = find_secondary_opd_tables(&data);
        assert!(
            tables.is_empty(),
            "mismatched seq bytes must not match; got {tables:?}"
        );
    }

    #[test]
    fn find_secondary_opd_tables_on_real_sshd_elf() {
        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.elf");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let tables = find_secondary_opd_tables(&data);
        // SSHD has two adjacent tables at guest 0x829b10 and 0x829b78.
        assert_eq!(tables.len(), 2, "SSHD must expose two secondary OPD tables");
        assert_eq!(tables[0].guest_addr, 0x829b10);
        assert_eq!(tables[0].size, SECONDARY_OPD_TABLE_SIZE);
        assert_eq!(tables[1].guest_addr, 0x829b78);
        assert_eq!(tables[1].size, SECONDARY_OPD_TABLE_SIZE);
    }

    #[test]
    fn find_secondary_opd_tables_on_real_wipeout_elf() {
        let path = std::path::PathBuf::from(
            "../../tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.elf",
        );
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let tables = find_secondary_opd_tables(&data);
        // WipEout has two adjacent tables at guest 0x925008 and 0x925070.
        assert_eq!(
            tables.len(),
            2,
            "WipEout must expose two secondary OPD tables"
        );
        assert_eq!(tables[0].guest_addr, 0x925008);
        assert_eq!(tables[1].guest_addr, 0x925070);
    }

    /// Build a 2-PT_LOAD ELF: exec segment at `[exec_off, +exec_sz)` /
    /// `exec_vaddr`, plus a non-executable segment at `[data_off, +data_sz)` /
    /// `data_vaddr` for the caller to plant a table into.
    fn mk_elf_with_exec_and_data_pt_loads(
        exec_off: usize,
        exec_sz: usize,
        exec_vaddr: u64,
        data_off: usize,
        data_sz: usize,
        data_vaddr: u64,
    ) -> Vec<u8> {
        let last = exec_off + exec_sz;
        let last = last.max(data_off + data_sz);
        let mut data = vec![0u8; last + 16];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&2u16.to_be_bytes());
        // Exec PT_LOAD (PF_X=1).
        let ph0 = 64;
        data[ph0..ph0 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph0 + 4..ph0 + 8].copy_from_slice(&1u32.to_be_bytes());
        data[ph0 + 8..ph0 + 16].copy_from_slice(&(exec_off as u64).to_be_bytes());
        data[ph0 + 16..ph0 + 24].copy_from_slice(&exec_vaddr.to_be_bytes());
        data[ph0 + 32..ph0 + 40].copy_from_slice(&(exec_sz as u64).to_be_bytes());
        data[ph0 + 40..ph0 + 48].copy_from_slice(&(exec_sz as u64).to_be_bytes());
        // Data PT_LOAD (PF_W=1, PF_R=1).
        let ph1 = 64 + 56;
        data[ph1..ph1 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[ph1 + 4..ph1 + 8].copy_from_slice(&6u32.to_be_bytes());
        data[ph1 + 8..ph1 + 16].copy_from_slice(&(data_off as u64).to_be_bytes());
        data[ph1 + 16..ph1 + 24].copy_from_slice(&data_vaddr.to_be_bytes());
        data[ph1 + 32..ph1 + 40].copy_from_slice(&(data_sz as u64).to_be_bytes());
        data[ph1 + 40..ph1 + 48].copy_from_slice(&(data_sz as u64).to_be_bytes());
        data
    }

    /// Plant an (id, ptr, opd_slot) row at `file_off` with the given
    /// code pointer in column 1.
    fn plant_indirect_row(data: &mut [u8], file_off: usize, id: u32, code_ptr: u32, opd: u32) {
        data[file_off..file_off + 4].copy_from_slice(&id.to_be_bytes());
        data[file_off + 4..file_off + 8].copy_from_slice(&code_ptr.to_be_bytes());
        data[file_off + 8..file_off + 12].copy_from_slice(&opd.to_be_bytes());
    }

    #[test]
    fn find_indirect_opd_tables_finds_a_4_row_run() {
        let exec_off = 0x200usize;
        let exec_sz = 0x200usize;
        let exec_vaddr = 0x1_0000u64;
        let data_off = 0x600usize;
        let data_sz = 0x100usize;
        let data_vaddr = 0x86_0000u64;
        let mut data = mk_elf_with_exec_and_data_pt_loads(
            exec_off, exec_sz, exec_vaddr, data_off, data_sz, data_vaddr,
        );
        // Plant four 12-byte rows where column 1 points into the exec segment.
        let table_off = data_off + 0x40;
        for i in 0..4 {
            plant_indirect_row(
                &mut data,
                table_off + i * 12,
                i as u32 + 1,
                exec_vaddr as u32 + (i as u32) * 8,
                0x00ae_eb80,
            );
        }
        let tables = find_indirect_opd_tables(&data);
        assert_eq!(tables.len(), 1, "exactly one table; got {tables:?}");
        assert_eq!(tables[0].guest_addr, data_vaddr + 0x40);
        assert_eq!(tables[0].size, 4 * INDIRECT_OPD_TABLE_STRIDE);
    }

    #[test]
    fn find_indirect_opd_tables_rejects_short_run() {
        let exec_off = 0x200usize;
        let exec_sz = 0x200usize;
        let exec_vaddr = 0x1_0000u64;
        let data_off = 0x600usize;
        let data_sz = 0x100usize;
        let data_vaddr = 0x86_0000u64;
        let mut data = mk_elf_with_exec_and_data_pt_loads(
            exec_off, exec_sz, exec_vaddr, data_off, data_sz, data_vaddr,
        );
        // Plant three rows; below MIN_ROWS threshold of 4. Random data
        // sometimes contains two consecutive pointer-shaped quads; the
        // 4-row minimum suppresses that class of false positive.
        let table_off = data_off + 0x40;
        for i in 0..3 {
            plant_indirect_row(
                &mut data,
                table_off + i * 12,
                i as u32 + 1,
                exec_vaddr as u32 + (i as u32) * 8,
                0,
            );
        }
        let tables = find_indirect_opd_tables(&data);
        assert!(tables.is_empty(), "3-row run is below threshold");
    }

    #[test]
    fn find_indirect_opd_tables_on_real_wipeout_elf() {
        let path = std::path::PathBuf::from(
            "../../tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.elf",
        );
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        let tables = find_indirect_opd_tables(&data);
        // Stage A.0 trace identified one indirect-OPD table at WipEout's
        // data offset 0xc1110 (guest 0x921110). The exact row count is
        // a function of WipEout's import count; assert it covers at
        // least the byte range Stage D's pending-bytes investigation
        // observed.
        let covering = tables
            .iter()
            .find(|t| t.guest_addr <= 0x921110 && t.guest_addr + t.size > 0x9213c8);
        assert!(
            covering.is_some(),
            "WipEout's indirect-OPD table at 0x921110 must be found; got {tables:?}",
        );
    }
}
