//! Parser and loader for decrypted PS3 firmware PRX (ELF64 type
//! 0xFFA4). Handles the firmware side of PRX loading; game-side
//! import parsing lives in [`crate::prx`].

use crate::loader;
use std::collections::BTreeMap;

// -- ELF constants --

/// PS3 PRX ELF type.
const ET_PRX: u16 = 0xFFA4;
/// PT_LOAD segment type.
const PT_LOAD: u32 = 1;
/// PS3 relocation segment type.
const PT_PRX_RELOC: u32 = 0x700000A4;
/// ELF64 header size.
const ELF_HEADER_SIZE: usize = 64;
/// ELF magic.
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

// -- Relocation type constants --

/// 32-bit absolute address.
pub const R_PPC64_ADDR32: u32 = 1;
/// Low 16 bits of address (ori immediate).
pub const R_PPC64_ADDR16_LO: u32 = 4;
/// High 16 bits of address (oris immediate, no adjust).
pub const R_PPC64_ADDR16_HI: u32 = 5;
/// High 16 bits adjusted (add 1 if bit 15 of full address is set).
pub const R_PPC64_ADDR16_HA: u32 = 6;

// -- Well-known system export NIDs --

/// NID for module_start in the system export entry.
const NID_MODULE_START: u32 = 0xbc9a0086;
/// NID for module_stop in the system export entry.
const NID_MODULE_STOP: u32 = 0xab779874;

// -- Public data types --

/// A parsed decrypted PRX module, ready for loading into guest memory.
#[derive(Debug, Clone)]
pub struct ParsedPrx {
    /// Module name from sys_prx_module_info_t (e.g., "liblv2").
    pub name: String,
    /// Table of Contents base address (unrelocated).
    pub toc: u32,
    /// Text (code) segment.
    pub text: PrxSegment,
    /// Data segment.
    pub data: PrxSegment,
    /// Exported libraries (non-system entries with NID tables).
    pub exports: Vec<PrxExportLib>,
    /// Relocation entries from the 0x700000A4 segment.
    pub relocations: Vec<PrxRelocation>,
    /// module_start function: (code_vaddr, toc) from OPD. None if absent.
    pub module_start: Option<PrxOpd>,
    /// module_stop function: (code_vaddr, toc) from OPD. None if absent.
    pub module_stop: Option<PrxOpd>,
}

/// A PT_LOAD segment's raw data and address info.
#[derive(Debug, Clone)]
pub struct PrxSegment {
    /// Virtual address (unrelocated, typically 0 for text).
    pub vaddr: u64,
    /// Size of data in file.
    pub filesz: u64,
    /// Size in memory (may be larger than filesz for BSS).
    pub memsz: u64,
    /// Raw segment bytes (filesz bytes, caller zero-extends to memsz).
    pub data: Vec<u8>,
}

/// An exported library within a PRX module.
#[derive(Debug, Clone)]
pub struct PrxExportLib {
    /// Library name (e.g., "sysPrxForUser", "cellSysmodule").
    pub name: String,
    /// Library attributes.
    pub attrs: u16,
    /// Exported functions: (NID, stub vaddr).
    pub functions: Vec<PrxExport>,
    /// Exported variables: (NID, vaddr).
    pub variables: Vec<PrxExport>,
}

/// A single exported symbol (function or variable).
#[derive(Debug, Clone, Copy)]
pub struct PrxExport {
    /// NID identifying the symbol.
    pub nid: u32,
    /// Virtual address of the symbol's OPD (functions) or data (variables).
    /// Unrelocated -- caller must add the base address.
    pub vaddr: u32,
}

/// Official Procedure Descriptor with its location and contents.
#[derive(Debug, Clone, Copy)]
pub struct PrxOpd {
    /// Virtual address of the OPD itself (unrelocated).
    pub opd_vaddr: u32,
    /// Code entry point read from the OPD (unrelocated).
    pub code: u32,
    /// Table of Contents base read from the OPD.
    pub toc: u32,
}

/// A single ELF64 RELA relocation entry.
///
/// The PS3 PRX `sym` field packs two segment indices:
/// `sym & 0xFF` is the target segment (0 = text, 1 = data) and
/// `(sym >> 8) & 0xFF` is the value segment the addend is relative to.
#[derive(Debug, Clone, Copy)]
pub struct PrxRelocation {
    /// Offset within the target segment to patch.
    pub offset: u64,
    /// Relocation type (R_PPC64_ADDR32, etc.).
    pub rtype: u32,
    /// Packed segment indices: (value_seg << 8) | target_seg.
    pub sym: u32,
    /// Signed addend, relative to the value segment's vaddr.
    pub addend: i64,
}

/// Why PRX parsing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrxParseError {
    /// File too small for an ELF header.
    TooSmall,
    /// Not a valid ELF file (bad magic).
    BadMagic,
    /// Not a 64-bit big-endian ELF.
    NotElf64Be,
    /// ELF type is not 0xFFA4 (PS3 PRX).
    NotPrx(u16),
    /// Fewer than 2 PT_LOAD segments.
    MissingSegments,
    /// A read went out of bounds.
    OutOfBounds,
    /// Module name not found in the binary.
    NoModuleInfo,
}

// -- Parsing --

/// Parse a decrypted PRX (ELF64 type 0xFFA4) into its components.
///
/// Input must already be decrypted (e.g. via RPCS3 `--decrypt`), not
/// a raw SCE-encrypted SELF.
pub fn parse_prx(data: &[u8]) -> Result<ParsedPrx, PrxParseError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(PrxParseError::TooSmall);
    }
    if data[0..4] != ELF_MAGIC {
        return Err(PrxParseError::BadMagic);
    }
    if data[4] != 2 || data[5] != 2 {
        return Err(PrxParseError::NotElf64Be);
    }
    let e_type = loader::read_u16(data, 16);
    if e_type != ET_PRX {
        return Err(PrxParseError::NotPrx(e_type));
    }

    let phoff = loader::read_u64(data, 32) as usize;
    let phentsize = loader::read_u16(data, 54) as usize;
    let phnum = loader::read_u16(data, 56) as usize;
    // ELF64 phdr is 56 bytes; we read fields up to base + 40 + 8 = 48.
    // phentsize < 48 would either OOB-read into adjacent entries or, if
    // 0, duplicate-read the same phdr every iteration.
    if phentsize < 56 {
        return Err(PrxParseError::OutOfBounds);
    }

    let mut loads: Vec<RawPhdr> = Vec::new();
    let mut reloc_phdr: Option<RawPhdr> = None;

    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            return Err(PrxParseError::OutOfBounds);
        }
        let p_type = loader::read_u32(data, base);
        let p_offset = loader::read_u64(data, base + 8) as usize;
        let p_vaddr = loader::read_u64(data, base + 16);
        let p_paddr = loader::read_u64(data, base + 24);
        let p_filesz = loader::read_u64(data, base + 32);
        let p_memsz = loader::read_u64(data, base + 40);

        let phdr = RawPhdr {
            p_type,
            p_offset,
            p_vaddr,
            p_paddr,
            p_filesz,
            p_memsz,
        };

        match p_type {
            PT_LOAD => loads.push(phdr),
            PT_PRX_RELOC => reloc_phdr = Some(phdr),
            _ => {}
        }
    }

    if loads.len() < 2 {
        return Err(PrxParseError::MissingSegments);
    }

    let seg_map: Vec<SegEntry> = loads
        .iter()
        .filter(|l| l.p_filesz > 0)
        .map(|l| SegEntry {
            vaddr: l.p_vaddr as usize,
            file_offset: l.p_offset,
            size: l.p_filesz as usize,
        })
        .collect();

    let text = extract_segment(data, &loads[0])?;
    let data_seg = extract_segment(data, &loads[1])?;

    // PT_LOAD[0].paddr doubles as the file offset of module_info.
    let mi_file_off = loads[0].p_paddr as usize;
    let (name, toc, exports_range, _imports_range) = parse_module_info(data, mi_file_off)?;

    let exports = parse_export_table(data, &seg_map, exports_range)?;

    let module_start = find_system_opd(data, &seg_map, &exports_range, NID_MODULE_START)?;
    let module_stop = find_system_opd(data, &seg_map, &exports_range, NID_MODULE_STOP)?;

    let relocations = match reloc_phdr {
        Some(rp) => parse_relocations(data, &rp)?,
        None => Vec::new(),
    };

    Ok(ParsedPrx {
        name,
        toc,
        text,
        data: data_seg,
        exports,
        relocations,
        module_start,
        module_stop,
    })
}

// -- Internal helpers --

struct RawPhdr {
    #[allow(dead_code)]
    p_type: u32,
    p_offset: usize,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
}

struct SegEntry {
    vaddr: usize,
    file_offset: usize,
    size: usize,
}

fn v2f(seg_map: &[SegEntry], vaddr: usize) -> Option<usize> {
    for seg in seg_map {
        if vaddr >= seg.vaddr && vaddr < seg.vaddr + seg.size {
            return Some(vaddr - seg.vaddr + seg.file_offset);
        }
    }
    None
}

fn extract_segment(data: &[u8], phdr: &RawPhdr) -> Result<PrxSegment, PrxParseError> {
    let end = phdr.p_offset + phdr.p_filesz as usize;
    if end > data.len() {
        return Err(PrxParseError::OutOfBounds);
    }
    Ok(PrxSegment {
        vaddr: phdr.p_vaddr,
        filesz: phdr.p_filesz,
        memsz: phdr.p_memsz,
        data: data[phdr.p_offset..end].to_vec(),
    })
}

/// Parse `sys_prx_module_info_t` at `file_off`.
///
/// Layout: `+0` u16 attributes, `+2` u8[2] version, `+4` char[28] name,
/// `+32` u32 toc, `+36/+40` u32 exports_{start,end} (vaddr),
/// `+44/+48` u32 imports_{start,end} (vaddr).
fn parse_module_info(
    data: &[u8],
    file_off: usize,
) -> Result<(String, u32, VaddrRange, VaddrRange), PrxParseError> {
    if file_off + 52 > data.len() {
        return Err(PrxParseError::NoModuleInfo);
    }
    let name_bytes = &data[file_off + 4..file_off + 32];
    let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(28);
    let name = String::from_utf8_lossy(&name_bytes[..name_end]).into_owned();
    if name.is_empty() || !name.is_ascii() {
        return Err(PrxParseError::NoModuleInfo);
    }

    let toc = loader::read_u32(data, file_off + 32);
    let exp_start = loader::read_u32(data, file_off + 36);
    let exp_end = loader::read_u32(data, file_off + 40);
    let imp_start = loader::read_u32(data, file_off + 44);
    let imp_end = loader::read_u32(data, file_off + 48);

    Ok((
        name,
        toc,
        VaddrRange {
            start: exp_start,
            end: exp_end,
        },
        VaddrRange {
            start: imp_start,
            end: imp_end,
        },
    ))
}

#[derive(Debug, Clone, Copy)]
struct VaddrRange {
    start: u32,
    end: u32,
}

/// Export entry size field.
const EXPORT_ENTRY_MIN_SIZE: u8 = 0x1C; // 28 bytes

/// System export attribute flag.
const EXPORT_ATTR_SYSTEM: u16 = 0x8000;

/// Parse the export table into a list of non-system export libraries.
fn parse_export_table(
    data: &[u8],
    seg_map: &[SegEntry],
    range: VaddrRange,
) -> Result<Vec<PrxExportLib>, PrxParseError> {
    if range.start >= range.end {
        return Ok(Vec::new());
    }
    let size = (range.end - range.start) as usize;
    if size > 0x10000 {
        return Err(PrxParseError::OutOfBounds);
    }

    let start_foff = v2f(seg_map, range.start as usize).ok_or(PrxParseError::OutOfBounds)?;
    // Compute end_foff from the size rather than translating range.end
    // through v2f: range.end is exclusive and may equal seg.vaddr +
    // seg.size for a table that fills its segment to the boundary, in
    // which case v2f's strict-less-than upper-bound check returns None.
    let end_foff = start_foff + size;

    let mut libs = Vec::new();
    let mut pos = start_foff;

    while pos < end_foff {
        if pos >= data.len() {
            break;
        }
        let entry_size = data[pos];
        if entry_size < EXPORT_ENTRY_MIN_SIZE {
            break;
        }
        let entry_size = entry_size as usize;
        if pos + entry_size > data.len() {
            return Err(PrxParseError::OutOfBounds);
        }

        let attrs = loader::read_u16(data, pos + 4);
        let num_func = loader::read_u16(data, pos + 6) as usize;
        let num_var = loader::read_u16(data, pos + 8) as usize;
        let lib_name_ptr = loader::read_u32(data, pos + 16);
        let nid_table_ptr = loader::read_u32(data, pos + 20);
        let stub_table_ptr = loader::read_u32(data, pos + 24);

        if (attrs & EXPORT_ATTR_SYSTEM) == 0 {
            let lib_name = if lib_name_ptr != 0 {
                read_cstring(data, seg_map, lib_name_ptr as usize)
            } else {
                String::new()
            };

            let total = num_func + num_var;
            let (functions, variables) = read_export_entries(
                data,
                seg_map,
                nid_table_ptr,
                stub_table_ptr,
                num_func,
                total,
            )?;

            libs.push(PrxExportLib {
                name: lib_name,
                attrs,
                functions,
                variables,
            });
        }

        pos += entry_size;
    }

    Ok(libs)
}

/// Read the NID and stub tables into `(functions, variables)`;
/// entries at `[0, num_func)` are functions, the remainder variables.
fn read_export_entries(
    data: &[u8],
    seg_map: &[SegEntry],
    nid_ptr: u32,
    stub_ptr: u32,
    num_func: usize,
    total: usize,
) -> Result<(Vec<PrxExport>, Vec<PrxExport>), PrxParseError> {
    if total == 0 || nid_ptr == 0 {
        return Ok((Vec::new(), Vec::new()));
    }

    let nid_foff = v2f(seg_map, nid_ptr as usize).ok_or(PrxParseError::OutOfBounds)?;
    let stub_foff = v2f(seg_map, stub_ptr as usize).ok_or(PrxParseError::OutOfBounds)?;

    let mut functions = Vec::with_capacity(num_func);
    let mut variables = Vec::with_capacity(total - num_func);

    for i in 0..total {
        let n_off = nid_foff + i * 4;
        let s_off = stub_foff + i * 4;
        if n_off + 4 > data.len() || s_off + 4 > data.len() {
            return Err(PrxParseError::OutOfBounds);
        }
        let nid = loader::read_u32(data, n_off);
        let vaddr = loader::read_u32(data, s_off);
        let entry = PrxExport { nid, vaddr };
        if i < num_func {
            functions.push(entry);
        } else {
            variables.push(entry);
        }
    }

    Ok((functions, variables))
}

/// Find the OPD for a well-known NID in the system export entry.
fn find_system_opd(
    data: &[u8],
    seg_map: &[SegEntry],
    exports_range: &VaddrRange,
    target_nid: u32,
) -> Result<Option<PrxOpd>, PrxParseError> {
    if exports_range.start >= exports_range.end {
        return Ok(None);
    }

    let start_foff =
        v2f(seg_map, exports_range.start as usize).ok_or(PrxParseError::OutOfBounds)?;
    // See parse_export_table: compute end_foff from the size to handle
    // tables that fill a segment to its exclusive boundary.
    let end_foff = start_foff + (exports_range.end - exports_range.start) as usize;

    let mut pos = start_foff;
    while pos < end_foff {
        if pos >= data.len() {
            break;
        }
        let entry_size = data[pos];
        if entry_size < EXPORT_ENTRY_MIN_SIZE {
            break;
        }
        let entry_size = entry_size as usize;
        if pos + entry_size > data.len() {
            break;
        }

        let attrs = loader::read_u16(data, pos + 4);
        if (attrs & EXPORT_ATTR_SYSTEM) != 0 {
            let num_func = loader::read_u16(data, pos + 6) as usize;
            let nid_table_ptr = loader::read_u32(data, pos + 20);
            let stub_table_ptr = loader::read_u32(data, pos + 24);

            if nid_table_ptr != 0 {
                let nid_foff =
                    v2f(seg_map, nid_table_ptr as usize).ok_or(PrxParseError::OutOfBounds)?;
                let stub_foff =
                    v2f(seg_map, stub_table_ptr as usize).ok_or(PrxParseError::OutOfBounds)?;

                for i in 0..num_func {
                    let n_off = nid_foff + i * 4;
                    if n_off + 4 > data.len() {
                        break;
                    }
                    let nid = loader::read_u32(data, n_off);
                    if nid == target_nid {
                        let opd_vaddr = loader::read_u32(data, stub_foff + i * 4) as usize;
                        let opd_foff = v2f(seg_map, opd_vaddr).ok_or(PrxParseError::OutOfBounds)?;
                        if opd_foff + 8 > data.len() {
                            return Err(PrxParseError::OutOfBounds);
                        }
                        let code = loader::read_u32(data, opd_foff);
                        let toc = loader::read_u32(data, opd_foff + 4);
                        return Ok(Some(PrxOpd {
                            opd_vaddr: opd_vaddr as u32,
                            code,
                            toc,
                        }));
                    }
                }
            }
        }

        pos += entry_size;
    }

    Ok(None)
}

/// Parse RELA entries from the 0x700000A4 relocation segment.
fn parse_relocations(data: &[u8], phdr: &RawPhdr) -> Result<Vec<PrxRelocation>, PrxParseError> {
    let start = phdr.p_offset;
    let size = phdr.p_filesz as usize;
    let end = start + size;
    if end > data.len() {
        return Err(PrxParseError::OutOfBounds);
    }

    const RELA_SIZE: usize = 24;
    let count = size / RELA_SIZE;
    let mut relocs = Vec::with_capacity(count);

    for i in 0..count {
        let off = start + i * RELA_SIZE;
        let r_offset = loader::read_u64(data, off);
        let r_info = loader::read_u64(data, off + 8);
        let r_addend = loader::read_u64(data, off + 16) as i64;
        let r_sym = (r_info >> 32) as u32;
        let r_type = (r_info & 0xFFFF_FFFF) as u32;

        relocs.push(PrxRelocation {
            offset: r_offset,
            rtype: r_type,
            sym: r_sym,
            addend: r_addend,
        });
    }

    Ok(relocs)
}

fn read_cstring(data: &[u8], seg_map: &[SegEntry], vaddr: usize) -> String {
    // Failed lookups embed the vaddr so a corrupt name pointer is
    // visible in the parsed module rather than indistinguishable from
    // a legitimately-empty string.
    let foff = match v2f(seg_map, vaddr) {
        Some(o) => o,
        None => return format!("<unmapped:0x{vaddr:x}>"),
    };
    if foff >= data.len() {
        return format!("<oob:0x{vaddr:x}>");
    }
    let end = data[foff..]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(data.len() - foff);
    String::from_utf8_lossy(&data[foff..foff + end]).into_owned()
}

// -- Loading into guest memory --

/// A PRX module loaded into guest memory with relocations applied.
#[derive(Debug, Clone)]
pub struct LoadedPrx {
    /// Module name.
    pub name: String,
    /// Base address in guest memory.
    pub base: u64,
    /// Relocated TOC value (base + unrelocated toc).
    pub toc: u64,
    /// Text segment range in guest memory: [start, start + memsz).
    pub text_start: u64,
    /// End of text segment in guest memory.
    pub text_end: u64,
    /// Data segment range in guest memory.
    pub data_start: u64,
    /// End of data segment in guest memory.
    pub data_end: u64,
    /// Exported function NIDs mapped to relocated OPD guest addresses.
    pub exports: BTreeMap<u32, u64>,
    /// module_start entry point, computed from base + parsed OPD. Not
    /// every OPD field has a relocation entry, so this is derived
    /// rather than read back from guest memory.
    pub module_start: Option<LoadedOpd>,
    /// module_stop entry point.
    pub module_stop: Option<LoadedOpd>,
    /// Number of relocations applied.
    pub relocs_applied: usize,
}

/// A relocated OPD entry, ready for setting up PPU state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadedOpd {
    /// Code entry point (relocated guest address).
    pub code: u64,
    /// Table of Contents base (relocated guest address).
    pub toc: u64,
}

/// Why loading a PRX into guest memory failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrxLoadError {
    /// A segment does not fit in guest memory at the chosen base.
    SegmentOutOfRange {
        /// Start address attempted.
        guest_addr: u64,
        /// Segment size.
        size: u64,
    },
    /// Guest memory write failed.
    MemoryWrite(u64),
    /// An unsupported relocation type was encountered.
    UnsupportedReloc(u32),
    /// A relocation referenced a segment index outside the loaded
    /// `[text, data]` pair. PS3 PRX modules have at most 2 PT_LOAD
    /// segments today, so any higher index indicates corruption or a
    /// firmware shape the loader does not yet model.
    RelocSegmentOutOfRange {
        /// The packed `r_sym` value of the offending relocation.
        sym: u32,
        /// The decoded segment index that exceeded the table.
        seg: usize,
    },
}

/// Load a parsed PRX at `base` and apply relocations. `base` must be
/// page-aligned and above the game's own memory footprint.
pub fn load_prx(
    prx: &ParsedPrx,
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
) -> Result<LoadedPrx, PrxLoadError> {
    let mem_size = memory.size();

    write_segment(memory, base, &prx.text, mem_size)?;
    write_segment(memory, base, &prx.data, mem_size)?;

    // seg_vaddrs indexed by segment number: 0 = text, 1 = data.
    let seg_vaddrs = [prx.text.vaddr, prx.data.vaddr];
    let relocs_applied = apply_relocations(memory, base, &seg_vaddrs, &prx.relocations)?;

    let mut exports = BTreeMap::new();
    for lib in &prx.exports {
        for func in &lib.functions {
            exports.insert(func.nid, base + func.vaddr as u64);
        }
    }

    // Derive module_start/stop from parsed OPD + base: the code field
    // is often not covered by the reloc table, so reading it back from
    // guest memory after relocation is unreliable. opd.code and opd.toc
    // are both unrelocated absolute vaddrs (within the PRX's address
    // space), so adding text.vaddr again would double-count for any
    // module whose text segment is not at vaddr 0. opd.toc is honored
    // per entry point rather than collapsed onto module_info's toc:
    // every shipping firmware module has the two equal, but the
    // per-OPD field exists to allow divergence and we read what the
    // OPD actually says.
    let module_start = prx.module_start.map(|opd| LoadedOpd {
        code: base + opd.code as u64,
        toc: base + opd.toc as u64,
    });
    let module_stop = prx.module_stop.map(|opd| LoadedOpd {
        code: base + opd.code as u64,
        toc: base + opd.toc as u64,
    });

    Ok(LoadedPrx {
        name: prx.name.clone(),
        base,
        toc: base + prx.toc as u64,
        text_start: base + prx.text.vaddr,
        text_end: base + prx.text.vaddr + prx.text.memsz,
        data_start: base + prx.data.vaddr,
        data_end: base + prx.data.vaddr + prx.data.memsz,
        exports,
        module_start,
        module_stop,
        relocs_applied,
    })
}

fn write_segment(
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
    seg: &PrxSegment,
    mem_size: u64,
) -> Result<(), PrxLoadError> {
    let guest_addr = base + seg.vaddr;
    let total_size = seg.memsz;

    if guest_addr + total_size > mem_size {
        return Err(PrxLoadError::SegmentOutOfRange {
            guest_addr,
            size: total_size,
        });
    }

    if !seg.data.is_empty() {
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(guest_addr), seg.filesz);
        if let Some(range) = range {
            memory
                .apply_commit(range, &seg.data)
                .map_err(|_| PrxLoadError::MemoryWrite(guest_addr))?;
        }
    }

    let bss_size = seg.memsz.saturating_sub(seg.filesz);
    if bss_size > 0 {
        let bss_addr = guest_addr + seg.filesz;
        let zeros = vec![0u8; bss_size as usize];
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(bss_addr), bss_size);
        if let Some(range) = range {
            memory
                .apply_commit(range, &zeros)
                .map_err(|_| PrxLoadError::MemoryWrite(bss_addr))?;
        }
    }

    Ok(())
}

/// Apply every relocation. `seg_vaddrs` is `[text_vaddr, data_vaddr]`.
fn apply_relocations(
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
    seg_vaddrs: &[u64],
    relocs: &[PrxRelocation],
) -> Result<usize, PrxLoadError> {
    let mut count = 0;
    for r in relocs {
        let target_seg = (r.sym & 0xFF) as usize;
        let value_seg = ((r.sym >> 8) & 0xFF) as usize;

        let target_base =
            seg_vaddrs
                .get(target_seg)
                .copied()
                .ok_or(PrxLoadError::RelocSegmentOutOfRange {
                    sym: r.sym,
                    seg: target_seg,
                })?;
        let value_base =
            seg_vaddrs
                .get(value_seg)
                .copied()
                .ok_or(PrxLoadError::RelocSegmentOutOfRange {
                    sym: r.sym,
                    seg: value_seg,
                })?;

        let target = base + target_base + r.offset;
        let value = (base + value_base).wrapping_add(r.addend as u64);
        // PS3 PRX uses PPC32-style 16-bit halves for ADDR16_LO/HI/HA;
        // mask to u32 so a base above 4 GiB does not silently truncate
        // bits 32..47 into the halfword. ADDR32 already truncates by
        // the cast.
        let value32 = value as u32;

        match r.rtype {
            R_PPC64_ADDR32 => {
                write_u32(memory, target, value32)?;
            }
            R_PPC64_ADDR16_LO => {
                write_u16(memory, target, value32 as u16)?;
            }
            R_PPC64_ADDR16_HI => {
                write_u16(memory, target, (value32 >> 16) as u16)?;
            }
            R_PPC64_ADDR16_HA => {
                // +0x8000 before shift: cancels sign extension of the paired LO.
                let ha = (value32.wrapping_add(0x8000) >> 16) as u16;
                write_u16(memory, target, ha)?;
            }
            other => return Err(PrxLoadError::UnsupportedReloc(other)),
        }

        count += 1;
    }
    Ok(count)
}

fn write_u32(
    memory: &mut cellgov_mem::GuestMemory,
    addr: u64,
    value: u32,
) -> Result<(), PrxLoadError> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4)
        .ok_or(PrxLoadError::MemoryWrite(addr))?;
    memory
        .apply_commit(range, &value.to_be_bytes())
        .map_err(|_| PrxLoadError::MemoryWrite(addr))
}

fn write_u16(
    memory: &mut cellgov_mem::GuestMemory,
    addr: u64,
    value: u16,
) -> Result<(), PrxLoadError> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 2)
        .ok_or(PrxLoadError::MemoryWrite(addr))?;
    memory
        .apply_commit(range, &value.to_be_bytes())
        .map_err(|_| PrxLoadError::MemoryWrite(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PRX ELF64 binary for testing.
    ///
    /// File layout: ELF header at 0, three 56-byte program headers at
    /// 0x40, text segment at 0x0F0 (vaddr 0x0), data segment at 0x1F0
    /// (vaddr 0x100, holds module_info, export tables, OPDs),
    /// relocation segment at 0x3F0.
    fn make_test_prx() -> Vec<u8> {
        let mut buf = vec![0u8; 0x500];

        buf[0..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 2;
        buf[5] = 2;
        buf[16..18].copy_from_slice(&ET_PRX.to_be_bytes());
        buf[32..40].copy_from_slice(&64u64.to_be_bytes());
        buf[54..56].copy_from_slice(&56u16.to_be_bytes());
        buf[56..58].copy_from_slice(&3u16.to_be_bytes());

        let phdr_base = 64;

        // PT_LOAD[0] text. paddr points to module_info inside the data segment.
        let ph0 = phdr_base;
        buf[ph0..ph0 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        buf[ph0 + 8..ph0 + 16].copy_from_slice(&0xF0u64.to_be_bytes());
        buf[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
        buf[ph0 + 24..ph0 + 32].copy_from_slice(&0x1F0u64.to_be_bytes());
        buf[ph0 + 32..ph0 + 40].copy_from_slice(&0x100u64.to_be_bytes());
        buf[ph0 + 40..ph0 + 48].copy_from_slice(&0x100u64.to_be_bytes());

        // PT_LOAD[1] data.
        let ph1 = phdr_base + 56;
        buf[ph1..ph1 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        buf[ph1 + 8..ph1 + 16].copy_from_slice(&0x1F0u64.to_be_bytes());
        buf[ph1 + 16..ph1 + 24].copy_from_slice(&0x100u64.to_be_bytes());
        buf[ph1 + 24..ph1 + 32].copy_from_slice(&0u64.to_be_bytes());
        buf[ph1 + 32..ph1 + 40].copy_from_slice(&0x200u64.to_be_bytes());
        buf[ph1 + 40..ph1 + 48].copy_from_slice(&0x200u64.to_be_bytes());

        // PT_PRX_RELOC (3 entries).
        let ph2 = phdr_base + 112;
        buf[ph2..ph2 + 4].copy_from_slice(&PT_PRX_RELOC.to_be_bytes());
        buf[ph2 + 8..ph2 + 16].copy_from_slice(&0x3F0u64.to_be_bytes());
        buf[ph2 + 32..ph2 + 40].copy_from_slice(&72u64.to_be_bytes());

        // Fill text with nops.
        for i in (0x0F0..0x1F0).step_by(4) {
            buf[i..i + 4].copy_from_slice(&0x6000_0000u32.to_be_bytes());
        }

        // module_info at file offset 0x1F0 (= PT_LOAD[0].paddr).
        let mi = 0x1F0;
        buf[mi..mi + 2].copy_from_slice(&0x0006u16.to_be_bytes());
        buf[mi + 2] = 1;
        buf[mi + 3] = 1;
        buf[mi + 4..mi + 11].copy_from_slice(b"testmod");
        buf[mi + 32..mi + 36].copy_from_slice(&0x200u32.to_be_bytes()); // toc
        buf[mi + 36..mi + 40].copy_from_slice(&0x130u32.to_be_bytes()); // exports_start
        buf[mi + 40..mi + 44].copy_from_slice(&0x168u32.to_be_bytes()); // exports_end
        buf[mi + 44..mi + 48].copy_from_slice(&0x168u32.to_be_bytes()); // imports_start
        buf[mi + 48..mi + 52].copy_from_slice(&0x168u32.to_be_bytes()); // imports_end

        // System export entry at vaddr 0x130 (file 0x220): 2 funcs + 1 var.
        let exp0 = 0x220;
        buf[exp0] = 0x1C;
        buf[exp0 + 4..exp0 + 6].copy_from_slice(&0x8000u16.to_be_bytes());
        buf[exp0 + 6..exp0 + 8].copy_from_slice(&2u16.to_be_bytes());
        buf[exp0 + 8..exp0 + 10].copy_from_slice(&1u16.to_be_bytes());
        buf[exp0 + 20..exp0 + 24].copy_from_slice(&0x1A0u32.to_be_bytes());
        buf[exp0 + 24..exp0 + 28].copy_from_slice(&0x1B0u32.to_be_bytes());

        // User export entry at vaddr 0x14C (file 0x23C): 3 funcs.
        let exp1 = exp0 + 28;
        buf[exp1] = 0x1C;
        buf[exp1 + 4..exp1 + 6].copy_from_slice(&0x0001u16.to_be_bytes());
        buf[exp1 + 6..exp1 + 8].copy_from_slice(&3u16.to_be_bytes());
        buf[exp1 + 16..exp1 + 20].copy_from_slice(&0x1C0u32.to_be_bytes());
        buf[exp1 + 20..exp1 + 24].copy_from_slice(&0x1D0u32.to_be_bytes());
        buf[exp1 + 24..exp1 + 28].copy_from_slice(&0x1E0u32.to_be_bytes());

        // System NID table (vaddr 0x1A0, file 0x290): module_start,
        // module_stop, and a variable NID.
        let nid0 = 0x290;
        buf[nid0..nid0 + 4].copy_from_slice(&NID_MODULE_START.to_be_bytes());
        buf[nid0 + 4..nid0 + 8].copy_from_slice(&NID_MODULE_STOP.to_be_bytes());
        buf[nid0 + 8..nid0 + 12].copy_from_slice(&0xD7F43016u32.to_be_bytes());

        // System stub table (vaddr 0x1B0, file 0x2A0): OPD vaddrs.
        let stub0 = 0x2A0;
        buf[stub0..stub0 + 4].copy_from_slice(&0x1F0u32.to_be_bytes());
        buf[stub0 + 4..stub0 + 8].copy_from_slice(&0x1F8u32.to_be_bytes());

        // OPDs at vaddr 0x1F0 / 0x1F8 (file 0x2E0 / 0x2E8).
        let opd_base = 0x2E0;
        buf[opd_base..opd_base + 4].copy_from_slice(&0x10u32.to_be_bytes());
        buf[opd_base + 4..opd_base + 8].copy_from_slice(&0x200u32.to_be_bytes());
        buf[opd_base + 8..opd_base + 12].copy_from_slice(&0x20u32.to_be_bytes());
        buf[opd_base + 12..opd_base + 16].copy_from_slice(&0x200u32.to_be_bytes());

        // Library name "testlib" (vaddr 0x1C0, file 0x2B0).
        buf[0x2B0..0x2B7].copy_from_slice(b"testlib");

        // User NID table (vaddr 0x1D0, file 0x2C0).
        let nid1 = 0x2C0;
        buf[nid1..nid1 + 4].copy_from_slice(&0xAAAAAAAAu32.to_be_bytes());
        buf[nid1 + 4..nid1 + 8].copy_from_slice(&0xBBBBBBBBu32.to_be_bytes());
        buf[nid1 + 8..nid1 + 12].copy_from_slice(&0xCCCCCCCCu32.to_be_bytes());

        // User stub table (vaddr 0x1E0, file 0x2D0).
        let stub1 = 0x2D0;
        buf[stub1..stub1 + 4].copy_from_slice(&0x40u32.to_be_bytes());
        buf[stub1 + 4..stub1 + 8].copy_from_slice(&0x50u32.to_be_bytes());
        buf[stub1 + 8..stub1 + 12].copy_from_slice(&0x60u32.to_be_bytes());

        // Three RELA entries (24 bytes each) at 0x3F0.
        let rel0 = 0x3F0;
        // ADDR32 text->text at offset 0x50, addend 0x80.
        buf[rel0..rel0 + 8].copy_from_slice(&0x50u64.to_be_bytes());
        let r_info0: u64 = R_PPC64_ADDR32 as u64;
        buf[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
        buf[rel0 + 16..rel0 + 24].copy_from_slice(&0x80i64.to_be_bytes());

        // ADDR16_HA text->text at offset 0x54, addend 0x200.
        let rel1 = rel0 + 24;
        buf[rel1..rel1 + 8].copy_from_slice(&0x54u64.to_be_bytes());
        let r_info1: u64 = R_PPC64_ADDR16_HA as u64;
        buf[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
        buf[rel1 + 16..rel1 + 24].copy_from_slice(&0x200i64.to_be_bytes());

        // ADDR32 target=data value=text at data-relative 0xF0 (OPD
        // code field of module_start), addend 0x10.
        let rel2 = rel1 + 24;
        buf[rel2..rel2 + 8].copy_from_slice(&0xF0u64.to_be_bytes());
        let r_info2: u64 = (0x0001u64 << 32) | R_PPC64_ADDR32 as u64;
        buf[rel2 + 8..rel2 + 16].copy_from_slice(&r_info2.to_be_bytes());
        buf[rel2 + 16..rel2 + 24].copy_from_slice(&0x10i64.to_be_bytes());

        buf
    }

    #[test]
    fn parse_test_prx_basic() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        assert_eq!(prx.name, "testmod");
        assert_eq!(prx.toc, 0x200);
    }

    #[test]
    fn parse_test_prx_segments() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        assert_eq!(prx.text.vaddr, 0);
        assert_eq!(prx.text.filesz, 0x100);
        assert_eq!(prx.data.vaddr, 0x100);
        assert_eq!(prx.data.filesz, 0x200);
    }

    #[test]
    fn parse_test_prx_exports() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        // Only the user library survives; the system entry is filtered.
        assert_eq!(prx.exports.len(), 1);
        assert_eq!(prx.exports[0].name, "testlib");
        assert_eq!(prx.exports[0].functions.len(), 3);
        assert_eq!(prx.exports[0].functions[0].nid, 0xAAAAAAAA);
        assert_eq!(prx.exports[0].functions[1].nid, 0xBBBBBBBB);
        assert_eq!(prx.exports[0].functions[2].nid, 0xCCCCCCCC);
        assert_eq!(prx.exports[0].functions[0].vaddr, 0x40);
    }

    #[test]
    fn parse_test_prx_module_start_stop() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let ms = prx.module_start.expect("module_start should be present");
        assert_eq!(ms.opd_vaddr, 0x1F0);
        assert_eq!(ms.code, 0x10);
        assert_eq!(ms.toc, 0x200);

        let mstop = prx.module_stop.expect("module_stop should be present");
        assert_eq!(mstop.opd_vaddr, 0x1F8);
        assert_eq!(mstop.code, 0x20);
        assert_eq!(mstop.toc, 0x200);
    }

    #[test]
    fn parse_test_prx_relocations() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        assert_eq!(prx.relocations.len(), 3);
        assert_eq!(prx.relocations[0].offset, 0x50);
        assert_eq!(prx.relocations[0].rtype, R_PPC64_ADDR32);
        assert_eq!(prx.relocations[0].sym, 0);
        assert_eq!(prx.relocations[0].addend, 0x80);
        assert_eq!(prx.relocations[1].offset, 0x54);
        assert_eq!(prx.relocations[1].rtype, R_PPC64_ADDR16_HA);
        assert_eq!(prx.relocations[1].addend, 0x200);
        assert_eq!(prx.relocations[2].offset, 0xF0);
        assert_eq!(prx.relocations[2].rtype, R_PPC64_ADDR32);
        assert_eq!(prx.relocations[2].sym, 0x0001);
        assert_eq!(prx.relocations[2].addend, 0x10);
    }

    #[test]
    fn reject_non_prx_elf() {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 2;
        // ET_EXEC, not PRX.
        data[16..18].copy_from_slice(&0x0002u16.to_be_bytes());
        assert!(matches!(parse_prx(&data), Err(PrxParseError::NotPrx(2))));
    }

    #[test]
    fn reject_too_small() {
        assert!(matches!(parse_prx(&[0; 10]), Err(PrxParseError::TooSmall)));
    }

    #[test]
    fn reject_bad_magic() {
        let data = vec![0u8; 128];
        assert!(matches!(parse_prx(&data), Err(PrxParseError::BadMagic)));
    }

    #[test]
    fn parse_real_liblv2() {
        let path = std::path::PathBuf::from(
            "../../tools/rpcs3/dev_flash_decrypted/sys/external/liblv2.prx",
        );
        if !path.exists() {
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let prx = parse_prx(&data).unwrap();

        assert_eq!(prx.name, "liblv2");
        assert_eq!(prx.toc, 0x1c620);

        let spy = prx
            .exports
            .iter()
            .find(|e| e.name == "sysPrxForUser")
            .expect("liblv2 should export sysPrxForUser");
        assert_eq!(spy.functions.len(), 157);

        let ms = prx.module_start.expect("liblv2 should have module_start");
        assert_eq!(ms.code, 0x0);
        assert_eq!(ms.toc, 0x1c620);

        assert!(
            prx.relocations.len() > 1000,
            "expected >1000 relocs, got {}",
            prx.relocations.len()
        );

        for r in &prx.relocations {
            assert!(
                matches!(
                    r.rtype,
                    R_PPC64_ADDR32 | R_PPC64_ADDR16_LO | R_PPC64_ADDR16_HI | R_PPC64_ADDR16_HA
                ),
                "unexpected reloc type {} at offset 0x{:x}",
                r.rtype,
                r.offset
            );
        }
    }

    #[test]
    fn load_test_prx_segments() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mem_size = 0x2000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(mem_size);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.name, "testmod");
        assert_eq!(loaded.base, base);
        assert_eq!(loaded.toc, base + 0x200);
        assert_eq!(loaded.text_start, base);
        assert_eq!(loaded.text_end, base + 0x100);
        assert_eq!(loaded.data_start, base + 0x100);
        assert_eq!(loaded.data_end, base + 0x300);
    }

    #[test]
    fn load_test_prx_exports_relocated() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.exports.len(), 3);
        assert_eq!(loaded.exports[&0xAAAAAAAA], base + 0x40);
        assert_eq!(loaded.exports[&0xBBBBBBBB], base + 0x50);
        assert_eq!(loaded.exports[&0xCCCCCCCC], base + 0x60);
    }

    #[test]
    fn load_test_prx_module_start_relocated() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(ms.code, base + 0x10);
        assert_eq!(ms.toc, base + 0x200);

        let mstop = loaded.module_stop.expect("module_stop");
        assert_eq!(mstop.code, base + 0x20);
        assert_eq!(mstop.toc, base + 0x200);
    }

    #[test]
    fn load_test_prx_relocations_applied() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.relocs_applied, 3);

        // ADDR32 text->text: target base+0x50, value base+0x80.
        let addr = (base + 0x50) as usize;
        let val = u32::from_be_bytes([
            mem.as_bytes()[addr],
            mem.as_bytes()[addr + 1],
            mem.as_bytes()[addr + 2],
            mem.as_bytes()[addr + 3],
        ]);
        assert_eq!(val, 0x1000_0080, "ADDR32 text->text mismatch");

        // ADDR16_HA: value 0x1000_0200, HA = (value + 0x8000) >> 16.
        let addr2 = (base + 0x54) as usize;
        let val2 = u16::from_be_bytes([mem.as_bytes()[addr2], mem.as_bytes()[addr2 + 1]]);
        assert_eq!(val2, 0x1000, "ADDR16_HA mismatch");

        // ADDR32 data->text patches module_start OPD code field.
        let addr3 = (base + 0x1F0) as usize;
        let val3 = u32::from_be_bytes([
            mem.as_bytes()[addr3],
            mem.as_bytes()[addr3 + 1],
            mem.as_bytes()[addr3 + 2],
            mem.as_bytes()[addr3 + 3],
        ]);
        assert_eq!(val3, 0x1000_0010, "ADDR32 data->text (OPD) mismatch");
    }

    #[test]
    fn load_test_prx_addr16_lo_and_hi() {
        let mut data = make_test_prx();

        // Shrink the relocation segment to 2 entries.
        let ph2 = 64 + 112;
        data[ph2 + 32..ph2 + 40].copy_from_slice(&48u64.to_be_bytes());

        let rel0 = 0x3F0;
        data[rel0..rel0 + 8].copy_from_slice(&0x58u64.to_be_bytes());
        let r_info0: u64 = R_PPC64_ADDR16_LO as u64;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
        data[rel0 + 16..rel0 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

        let rel1 = rel0 + 24;
        data[rel1..rel1 + 8].copy_from_slice(&0x5Au64.to_be_bytes());
        let r_info1: u64 = R_PPC64_ADDR16_HI as u64;
        data[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
        data[rel1 + 16..rel1 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();
        assert_eq!(loaded.relocs_applied, 2);

        // value = 0x1000_0000 + 0x12345678 = 0x2234_5678.
        let addr_lo = (base + 0x58) as usize;
        let lo = u16::from_be_bytes([mem.as_bytes()[addr_lo], mem.as_bytes()[addr_lo + 1]]);
        assert_eq!(lo, 0x5678, "ADDR16_LO mismatch");

        let addr_hi = (base + 0x5A) as usize;
        let hi = u16::from_be_bytes([mem.as_bytes()[addr_hi], mem.as_bytes()[addr_hi + 1]]);
        assert_eq!(hi, 0x2234, "ADDR16_HI mismatch");
    }

    #[test]
    fn load_prx_rejects_out_of_range() {
        let data = make_test_prx();
        let prx = parse_prx(&data).unwrap();

        let mut mem = cellgov_mem::GuestMemory::new(0x100);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(
            result,
            Err(PrxLoadError::SegmentOutOfRange { .. })
        ));
    }

    #[test]
    fn load_prx_rejects_reloc_with_out_of_range_segment() {
        // sym packs target_seg in low 8 bits and value_seg in next 8.
        // Set value_seg = 0x02 (third segment); the loader has only
        // [text, data] (2 entries), so this must error rather than
        // silently substitute segment 0's vaddr.
        let mut data = make_test_prx();
        let rel0 = 0x3F0;
        let r_info: u64 = (0x0200u64 << 32) | R_PPC64_ADDR32 as u64;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(
            result,
            Err(PrxLoadError::RelocSegmentOutOfRange { seg: 2, .. })
        ));
    }

    #[test]
    fn parse_rejects_phentsize_below_minimum() {
        // ELF64 phdr is 56 bytes; phentsize < 56 (including 0) means
        // either OOB-read or duplicate-read of the same phdr.
        let mut data = make_test_prx();
        data[54..56].copy_from_slice(&8u16.to_be_bytes());
        assert!(matches!(parse_prx(&data), Err(PrxParseError::OutOfBounds)));
    }

    #[test]
    fn read_cstring_unmapped_pointer_produces_diagnostic_string() {
        // Point the user library's name pointer at a vaddr no PT_LOAD
        // covers. read_cstring must surface the failing vaddr in the
        // returned name; returning "" silently hides the corruption
        // and produces export libraries with blank names downstream.
        let mut data = make_test_prx();
        let exp1 = 0x220 + 28; // user export entry
        let unmapped: u32 = 0xDEAD_0000;
        data[exp1 + 16..exp1 + 20].copy_from_slice(&unmapped.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.exports.len(), 1);
        assert!(
            prx.exports[0].name.starts_with("<unmapped:0x"),
            "expected diagnostic name for unmapped lib_name_ptr, got {:?}",
            prx.exports[0].name
        );
    }

    #[test]
    fn load_module_start_not_double_added_when_text_vaddr_nonzero() {
        // Synthesize a fixture where text.vaddr != 0 by patching the
        // PT_LOAD[0] vaddr field and shifting all the data-segment
        // pointers. opd.code is already an absolute vaddr in the PRX
        // address space, so ms.code must be `base + opd.code`. A
        // computation of `base + text.vaddr + opd.code` would
        // double-count text.vaddr whenever the segment is non-zero
        // based.
        let mut data = make_test_prx();
        // Set PT_LOAD[0].p_vaddr to 0x1000 (was 0).
        let ph0 = 64;
        let new_text_vaddr: u64 = 0x1000;
        data[ph0 + 16..ph0 + 24].copy_from_slice(&new_text_vaddr.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.text.vaddr, 0x1000);
        // opd.code is read as 0x10 from the OPD bytes regardless of
        // text.vaddr -- OPD code is an absolute PRX vaddr, not an
        // offset into the text segment.
        assert_eq!(
            prx.module_start.expect("module_start").code,
            0x10,
            "OPD code field is unrelocated absolute, not text-relative"
        );

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(
            ms.code,
            base + 0x10,
            "module_start.code must be base + opd.code, not base + text.vaddr + opd.code"
        );
    }

    #[test]
    fn load_uses_per_opd_toc_not_module_info_toc() {
        // Patch the module_start OPD's toc field to a value distinct
        // from module_info.toc. load_prx must honor the per-OPD toc
        // for module_start and module_stop entries; using
        // `base + prx.toc` unconditionally for both would silently
        // mask any divergence between the OPD's toc and the
        // module_info-level toc.
        let mut data = make_test_prx();
        let opd_base = 0x2E0;
        let alt_toc: u32 = 0x300;
        data[opd_base + 4..opd_base + 8].copy_from_slice(&alt_toc.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.toc, 0x200);
        assert_eq!(prx.module_start.expect("module_start").toc, alt_toc);

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(
            ms.toc,
            base + alt_toc as u64,
            "module_start.toc must come from the OPD's toc field, not module_info.toc"
        );
        // module_stop's OPD still carries 0x200, so it diverges from
        // module_start in this fixture and confirms per-OPD handling.
        let mstop = loaded.module_stop.expect("module_stop");
        assert_eq!(mstop.toc, base + 0x200);
    }

    #[test]
    fn load_prx_rejects_unsupported_reloc() {
        let mut data = make_test_prx();

        let rel0 = 0x3F0;
        let r_info: u64 = 99;
        data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let result = load_prx(&prx, &mut mem, 0x1000_0000);
        assert!(matches!(result, Err(PrxLoadError::UnsupportedReloc(99))));
    }

    #[test]
    fn load_real_liblv2() {
        let path = std::path::PathBuf::from(
            "../../tools/rpcs3/dev_flash_decrypted/sys/external/liblv2.prx",
        );
        if !path.exists() {
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let prx = parse_prx(&data).unwrap();

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        assert_eq!(loaded.name, "liblv2");
        assert_eq!(loaded.base, base);
        assert_eq!(loaded.toc, base + 0x1c620);
        assert!(loaded.relocs_applied > 1000);

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(ms.code, base, "module_start code should be at base");
        assert_eq!(ms.toc, base + 0x1c620, "module_start TOC");

        let text_start = base as usize;
        let first_insn = u32::from_be_bytes([
            mem.as_bytes()[text_start],
            mem.as_bytes()[text_start + 1],
            mem.as_bytes()[text_start + 2],
            mem.as_bytes()[text_start + 3],
        ]);
        let opcode = first_insn >> 26;
        assert!(
            opcode > 0 && opcode < 64,
            "first instruction should be valid PPC64, got 0x{:08x}",
            first_insn
        );

        assert!(
            loaded.exports.contains_key(&0x744680a2),
            "should export sys_initialize_tls"
        );
        assert!(
            loaded.exports.contains_key(&0xbdb18f83),
            "should export _sys_malloc"
        );
    }
}
