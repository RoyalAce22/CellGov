//! Parser and loader for decrypted PS3 firmware PRX (ELF64 type 0xFFA4).
//!
//! Game-side import parsing lives in [`crate::prx`].

use cellgov_ps3_abi::elf::{
    ELF_HEADER_SIZE, ELF_MAGIC, ET_PRX, NID_MODULE_START, NID_MODULE_STOP, PT_LOAD, PT_PRX_RELOC,
};

use crate::loader;
use std::collections::BTreeMap;

pub use cellgov_ps3_abi::elf::{
    R_PPC64_ADDR16_HA, R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO, R_PPC64_ADDR32,
};

/// Parsed decrypted PRX module ready for loading.
///
/// All vaddrs (`toc`, OPD fields, segment vaddrs) are unrelocated PRX-space
/// addresses; [`load_prx`] adds the chosen base.
#[derive(Debug, Clone)]
pub struct ParsedPrx {
    /// Module name from `sys_prx_module_info_t`.
    pub name: String,
    /// Module TOC vaddr (unrelocated).
    pub toc: u32,
    /// Text PT_LOAD segment.
    pub text: PrxSegment,
    /// Data PT_LOAD segment.
    pub data: PrxSegment,
    /// Non-system exported libraries.
    pub exports: Vec<PrxExportLib>,
    /// RELA entries from the PT_PRX_RELOC segment.
    pub relocations: Vec<PrxRelocation>,
    /// OPD for `module_start`, if exported.
    pub module_start: Option<PrxOpd>,
    /// OPD for `module_stop`, if exported.
    pub module_stop: Option<PrxOpd>,
}

/// PT_LOAD segment bytes plus its vaddr and sizes.
///
/// `data` holds `filesz` bytes; the loader zero-extends the
/// `memsz - filesz` BSS tail.
#[derive(Debug, Clone)]
pub struct PrxSegment {
    /// Unrelocated PRX-space vaddr of the segment.
    pub vaddr: u64,
    /// On-disk byte size.
    pub filesz: u64,
    /// In-memory byte size including BSS tail.
    pub memsz: u64,
    /// Raw `filesz` bytes from the file.
    pub data: Vec<u8>,
}

/// One exported library within a PRX module.
#[derive(Debug, Clone)]
pub struct PrxExportLib {
    /// Library name string.
    pub name: String,
    /// Library attribute flags.
    pub attrs: u16,
    /// Exported function entries.
    pub functions: Vec<PrxExport>,
    /// Exported variable entries.
    pub variables: Vec<PrxExport>,
}

/// One exported symbol; `vaddr` is unrelocated PRX-space.
#[derive(Debug, Clone, Copy)]
pub struct PrxExport {
    /// Symbol NID.
    pub nid: u32,
    /// Unrelocated PRX-space vaddr of the symbol's stub.
    pub vaddr: u32,
}

/// Official Procedure Descriptor: function entry point and TOC pair.
///
/// All three fields are unrelocated absolute PRX vaddrs, not segment-relative
/// offsets -- adding `text.vaddr` would double-count for non-zero-based text.
#[derive(Debug, Clone, Copy)]
pub struct PrxOpd {
    /// Vaddr of the OPD itself.
    pub opd_vaddr: u32,
    /// Function entry-point vaddr.
    pub code: u32,
    /// TOC vaddr paired with this entry point.
    pub toc: u32,
}

/// One ELF64 RELA relocation entry.
///
/// `sym` packs two segment indices: `sym & 0xFF` is the target segment to
/// patch (0 = text, 1 = data) and `(sym >> 8) & 0xFF` is the value segment
/// whose vaddr the `addend` is relative to.
#[derive(Debug, Clone, Copy)]
pub struct PrxRelocation {
    /// Offset within the target segment to patch.
    pub offset: u64,
    /// PPC64 relocation type code.
    pub rtype: u32,
    /// Packed target/value segment indices (low byte / next byte).
    pub sym: u32,
    /// Signed addend added to the value-segment vaddr.
    pub addend: i64,
}

/// Failure mode while parsing a decrypted PRX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrxParseError {
    /// Input is shorter than the ELF header.
    TooSmall,
    /// First four bytes are not the ELF magic.
    BadMagic,
    /// ELF class/encoding is not ELF64 big-endian.
    NotElf64Be,
    /// ELF e_type was not 0xFFA4 (PS3 PRX); carries the observed type.
    NotPrx(u16),
    /// Fewer than 2 PT_LOAD segments.
    MissingSegments,
    /// A computed file offset or size escaped the input buffer.
    OutOfBounds,
    /// `sys_prx_module_info_t` was missing or unreadable.
    NoModuleInfo,
}

/// Parse a decrypted PRX (ELF64 type 0xFFA4) into its components.
///
/// Input must already be decrypted (e.g. via RPCS3 `--decrypt`), not a raw
/// SCE-encrypted SELF.
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
    // ELF64 phdr is 56 bytes; smaller phentsize would alias entries.
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

    // PT_LOAD[0].p_paddr doubles as the file offset of module_info.
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
/// Layout: `+0` u16 attrs, `+2` u8[2] version, `+4` char[28] name, `+32` u32
/// toc, `+36/+40` u32 exports_{start,end} (vaddr), `+44/+48` u32
/// imports_{start,end} (vaddr).
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

use cellgov_ps3_abi::elf::{EXPORT_ATTR_SYSTEM, EXPORT_ENTRY_MIN_SIZE};

/// Walk the export table, returning every non-system library.
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
    // range.end is exclusive; v2f's strict-less-than would reject a table
    // whose end touches its segment boundary, so derive end_foff from size.
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

/// Read the NID and stub tables into `(functions, variables)`.
///
/// Entries at `[0, num_func)` are functions; the remainder are variables.
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
    // See [`parse_export_table`] for why end_foff comes from size, not v2f.
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
    // Failed lookups embed the vaddr so corrupt name pointers stay
    // distinguishable from legitimately-empty strings downstream.
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

/// PRX module loaded into guest memory with relocations applied.
///
/// Every address is post-relocation (already includes `base`). `module_start`
/// and `module_stop` are derived from the parsed OPD plus `base` rather than
/// read back from guest memory: not every OPD field has a relocation entry,
/// so a post-relocation read is unreliable.
#[derive(Debug, Clone)]
pub struct LoadedPrx {
    /// Module name from `sys_prx_module_info_t`.
    pub name: String,
    /// Guest base at which the module was loaded.
    pub base: u64,
    /// Relocated TOC guest address.
    pub toc: u64,
    /// Text segment range `[text_start, text_end)`.
    pub text_start: u64,
    /// Exclusive end of the text segment range.
    pub text_end: u64,
    /// Data segment range `[data_start, data_end)`.
    pub data_start: u64,
    /// Exclusive end of the data segment range.
    pub data_end: u64,
    /// Exported function NIDs mapped to relocated OPD guest addresses.
    pub exports: BTreeMap<u32, u64>,
    /// Relocated `module_start` OPD, if exported.
    pub module_start: Option<LoadedOpd>,
    /// Relocated `module_stop` OPD, if exported.
    pub module_stop: Option<LoadedOpd>,
    /// Number of relocation entries applied.
    pub relocs_applied: usize,
}

/// Relocated OPD entry; both fields are absolute guest addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadedOpd {
    /// Function entry-point guest address.
    pub code: u64,
    /// TOC guest address paired with this entry point.
    pub toc: u64,
}

/// Failure mode while loading a parsed PRX into guest memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrxLoadError {
    /// Segment does not fit in guest memory at the chosen base.
    SegmentOutOfRange {
        /// Guest address where the segment would have started.
        guest_addr: u64,
        /// Total in-memory size of the segment.
        size: u64,
    },
    /// Guest memory write failed at the given address.
    MemoryWrite(u64),
    /// Relocation type code is not handled by the loader.
    UnsupportedReloc(u32),
    /// Relocation referenced a segment index outside the loaded
    /// `[text, data]` pair (>= 2). Indicates corruption or a firmware
    /// shape the loader does not yet model.
    RelocSegmentOutOfRange {
        /// Raw `sym` field carrying the offending segment index.
        sym: u32,
        /// Decoded segment index that was out of range.
        seg: usize,
    },
}

/// Load a parsed PRX at `base` and apply relocations.
///
/// `base` must be page-aligned and above the game's own memory footprint.
pub fn load_prx(
    prx: &ParsedPrx,
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
) -> Result<LoadedPrx, PrxLoadError> {
    let mem_size = memory.size();

    write_segment(memory, base, &prx.text, mem_size)?;
    write_segment(memory, base, &prx.data, mem_size)?;

    // Indexed by sym-encoded segment number: 0 = text, 1 = data.
    let seg_vaddrs = [prx.text.vaddr, prx.data.vaddr];
    let relocs_applied = apply_relocations(memory, base, &seg_vaddrs, &prx.relocations)?;

    let mut exports = BTreeMap::new();
    for lib in &prx.exports {
        for func in &lib.functions {
            exports.insert(func.nid, base + func.vaddr as u64);
        }
    }

    // opd.code/toc are absolute PRX vaddrs; add base only. Per-OPD toc is
    // honored rather than collapsed onto module_info.toc -- shipping firmware
    // keeps them equal but the OPD field is authoritative if it diverges.
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

/// Apply every relocation; `seg_vaddrs` is `[text_vaddr, data_vaddr]`.
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
        // PRX ADDR16_{LO,HI,HA} are PPC32-style halves; truncate to u32 so a
        // base above 4 GiB does not bleed bits 32..47 into the halfword.
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
                // +0x8000 cancels sign-extension of the paired LO.
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

    /// Minimal PRX ELF64 fixture.
    ///
    /// Layout: ELF header at 0, three 56-byte program headers at 0x40, text
    /// at 0x0F0 (vaddr 0), data at 0x1F0 (vaddr 0x100, holds module_info,
    /// export tables, OPDs), relocations at 0x3F0.
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

        // PT_LOAD[0] text; p_paddr aliases module_info file offset.
        let ph0 = phdr_base;
        buf[ph0..ph0 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        buf[ph0 + 8..ph0 + 16].copy_from_slice(&0xF0u64.to_be_bytes());
        buf[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
        buf[ph0 + 24..ph0 + 32].copy_from_slice(&0x1F0u64.to_be_bytes());
        buf[ph0 + 32..ph0 + 40].copy_from_slice(&0x100u64.to_be_bytes());
        buf[ph0 + 40..ph0 + 48].copy_from_slice(&0x100u64.to_be_bytes());

        let ph1 = phdr_base + 56;
        buf[ph1..ph1 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        buf[ph1 + 8..ph1 + 16].copy_from_slice(&0x1F0u64.to_be_bytes());
        buf[ph1 + 16..ph1 + 24].copy_from_slice(&0x100u64.to_be_bytes());
        buf[ph1 + 24..ph1 + 32].copy_from_slice(&0u64.to_be_bytes());
        buf[ph1 + 32..ph1 + 40].copy_from_slice(&0x200u64.to_be_bytes());
        buf[ph1 + 40..ph1 + 48].copy_from_slice(&0x200u64.to_be_bytes());

        let ph2 = phdr_base + 112;
        buf[ph2..ph2 + 4].copy_from_slice(&PT_PRX_RELOC.to_be_bytes());
        buf[ph2 + 8..ph2 + 16].copy_from_slice(&0x3F0u64.to_be_bytes());
        buf[ph2 + 32..ph2 + 40].copy_from_slice(&72u64.to_be_bytes());

        // Fill text with nops.
        for i in (0x0F0..0x1F0).step_by(4) {
            buf[i..i + 4].copy_from_slice(&0x6000_0000u32.to_be_bytes());
        }

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

        let exp0 = 0x220;
        buf[exp0] = 0x1C;
        buf[exp0 + 4..exp0 + 6].copy_from_slice(&0x8000u16.to_be_bytes());
        buf[exp0 + 6..exp0 + 8].copy_from_slice(&2u16.to_be_bytes());
        buf[exp0 + 8..exp0 + 10].copy_from_slice(&1u16.to_be_bytes());
        buf[exp0 + 20..exp0 + 24].copy_from_slice(&0x1A0u32.to_be_bytes());
        buf[exp0 + 24..exp0 + 28].copy_from_slice(&0x1B0u32.to_be_bytes());

        let exp1 = exp0 + 28;
        buf[exp1] = 0x1C;
        buf[exp1 + 4..exp1 + 6].copy_from_slice(&0x0001u16.to_be_bytes());
        buf[exp1 + 6..exp1 + 8].copy_from_slice(&3u16.to_be_bytes());
        buf[exp1 + 16..exp1 + 20].copy_from_slice(&0x1C0u32.to_be_bytes());
        buf[exp1 + 20..exp1 + 24].copy_from_slice(&0x1D0u32.to_be_bytes());
        buf[exp1 + 24..exp1 + 28].copy_from_slice(&0x1E0u32.to_be_bytes());

        let nid0 = 0x290;
        buf[nid0..nid0 + 4].copy_from_slice(&NID_MODULE_START.to_be_bytes());
        buf[nid0 + 4..nid0 + 8].copy_from_slice(&NID_MODULE_STOP.to_be_bytes());
        buf[nid0 + 8..nid0 + 12].copy_from_slice(&0xD7F43016u32.to_be_bytes());

        let stub0 = 0x2A0;
        buf[stub0..stub0 + 4].copy_from_slice(&0x1F0u32.to_be_bytes());
        buf[stub0 + 4..stub0 + 8].copy_from_slice(&0x1F8u32.to_be_bytes());

        let opd_base = 0x2E0;
        buf[opd_base..opd_base + 4].copy_from_slice(&0x10u32.to_be_bytes());
        buf[opd_base + 4..opd_base + 8].copy_from_slice(&0x200u32.to_be_bytes());
        buf[opd_base + 8..opd_base + 12].copy_from_slice(&0x20u32.to_be_bytes());
        buf[opd_base + 12..opd_base + 16].copy_from_slice(&0x200u32.to_be_bytes());

        buf[0x2B0..0x2B7].copy_from_slice(b"testlib");

        let nid1 = 0x2C0;
        buf[nid1..nid1 + 4].copy_from_slice(&0xAAAAAAAAu32.to_be_bytes());
        buf[nid1 + 4..nid1 + 8].copy_from_slice(&0xBBBBBBBBu32.to_be_bytes());
        buf[nid1 + 8..nid1 + 12].copy_from_slice(&0xCCCCCCCCu32.to_be_bytes());

        let stub1 = 0x2D0;
        buf[stub1..stub1 + 4].copy_from_slice(&0x40u32.to_be_bytes());
        buf[stub1 + 4..stub1 + 8].copy_from_slice(&0x50u32.to_be_bytes());
        buf[stub1 + 8..stub1 + 12].copy_from_slice(&0x60u32.to_be_bytes());

        // Three RELA entries (24 bytes each) at 0x3F0.
        // [0] ADDR32 text->text at offset 0x50, addend 0x80.
        let rel0 = 0x3F0;
        buf[rel0..rel0 + 8].copy_from_slice(&0x50u64.to_be_bytes());
        let r_info0: u64 = R_PPC64_ADDR32 as u64;
        buf[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
        buf[rel0 + 16..rel0 + 24].copy_from_slice(&0x80i64.to_be_bytes());

        // [1] ADDR16_HA text->text at offset 0x54, addend 0x200.
        let rel1 = rel0 + 24;
        buf[rel1..rel1 + 8].copy_from_slice(&0x54u64.to_be_bytes());
        let r_info1: u64 = R_PPC64_ADDR16_HA as u64;
        buf[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
        buf[rel1 + 16..rel1 + 24].copy_from_slice(&0x200i64.to_be_bytes());

        // [2] ADDR32 target=data value=text at data+0xF0 (module_start
        // OPD code field), addend 0x10.
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
        data[16..18].copy_from_slice(&0x0002u16.to_be_bytes()); // ET_EXEC

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
        // value_seg = 0x02 against a 2-entry [text, data] table.
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
        let mut data = make_test_prx();
        data[54..56].copy_from_slice(&8u16.to_be_bytes());
        assert!(matches!(parse_prx(&data), Err(PrxParseError::OutOfBounds)));
    }

    #[test]
    fn read_cstring_unmapped_pointer_produces_diagnostic_string() {
        let mut data = make_test_prx();
        let exp1 = 0x220 + 28;
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
        let mut data = make_test_prx();
        let ph0 = 64;
        let new_text_vaddr: u64 = 0x1000;
        data[ph0 + 16..ph0 + 24].copy_from_slice(&new_text_vaddr.to_be_bytes());

        let prx = parse_prx(&data).unwrap();
        assert_eq!(prx.text.vaddr, 0x1000);
        assert_eq!(
            prx.module_start.expect("module_start").code,
            0x10,
            "OPD code is absolute PRX vaddr, not text-relative",
        );

        let base: u64 = 0x1000_0000;
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).unwrap();

        let ms = loaded.module_start.expect("module_start");
        assert_eq!(
            ms.code,
            base + 0x10,
            "ms.code = base + opd.code, not base + text.vaddr + opd.code",
        );
    }

    #[test]
    fn load_uses_per_opd_toc_not_module_info_toc() {
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
        assert_eq!(ms.toc, base + alt_toc as u64);
        // module_stop's OPD still carries 0x200; divergence proves per-OPD.
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
            loaded
                .exports
                .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::INITIALIZE_TLS),
            "should export sys_initialize_tls"
        );
        assert!(
            loaded
                .exports
                .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::MALLOC),
            "should export _sys_malloc"
        );
    }
}
