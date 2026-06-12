//! PPU ELF64 loader: copies PT_LOAD segments into guest memory and
//! resolves the entry-point OPD into `(pc, toc)`.

use crate::state::PpuState;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

pub(crate) use cellgov_mem::be::{read_u16, read_u32, read_u64};

/// `(addr, size)` pair describing where a segment would have been
/// placed. Shared by [`LoadError::SegmentOutOfRange`] (ELF PT_LOAD)
/// and [`crate::sprx::PrxLoadError::SegmentOutOfRange`] (PRX
/// text / data).
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

/// A PT_LOAD segment's address range and permission bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    /// Index of the program header within the ELF.
    pub index: usize,
    /// `p_offset`: file-relative byte position where the segment's
    /// initialized bytes begin. The loader has already bounds-checked
    /// the program-header slot the value was read from; callers that
    /// want a `[file_offset, file_offset + filesz)` range still need
    /// to validate the sum against the file length.
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
            file_offset: read_u64(data, base + 8),
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
/// range. WipEout's table at `data@0xc1110` is the driving
/// observation; SSHD-shaped titles may have a sibling table.
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
/// title's executable PT_LOAD range. Each detected run that meets the
/// internal row-count threshold is emitted as a single
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
#[path = "tests/loader_tests.rs"]
mod tests;
