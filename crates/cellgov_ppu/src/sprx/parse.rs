//! Decrypted-PRX (ELF64 type 0xFFA4) byte-level parser.
//!
//! Produces a [`ParsedPrx`] that [`crate::sprx::load_prx`] consumes; no
//! guest-memory dependency lives in this layer.

use cellgov_ps3_abi::elf::{
    ELF64_RELA_SIZE, ELF_HEADER_SIZE, ELF_MAGIC, ET_PRX, EXPORT_ATTR_SYSTEM, EXPORT_ENTRY_MIN_SIZE,
    NID_MODULE_START, NID_MODULE_STOP, PT_LOAD, PT_PRX_RELOC,
};

use crate::loader;

/// Parsed decrypted PRX module ready for loading.
///
/// All vaddrs (`toc`, OPD fields, segment vaddrs) are unrelocated PRX-space
/// addresses; [`crate::sprx::load_prx`] adds the chosen base.
#[derive(Debug, Clone)]
pub struct ParsedPrx {
    /// Module name from `sys_prx_module_info_t`.
    pub name: String,
    /// Stable id derived from [`Self::name`] via FNV-1a-32.
    pub module_id: crate::prx_loader::PrxModuleId,
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
        let base = i
            .checked_mul(phentsize)
            .and_then(|off| phoff.checked_add(off))
            .ok_or(PrxParseError::OutOfBounds)?;
        let end = base
            .checked_add(phentsize)
            .ok_or(PrxParseError::OutOfBounds)?;
        if end > data.len() {
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

    let module_id = crate::prx_loader::graph::module_id_from_name(&name);
    Ok(ParsedPrx {
        name,
        module_id,
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

#[derive(Debug, Clone, Copy)]
struct VaddrRange {
    start: u32,
    end: u32,
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
    // ELF requires p_memsz >= p_filesz. The loader sizes its region
    // check against memsz, so filesz > memsz would write past the
    // validated range.
    if phdr.p_filesz > phdr.p_memsz {
        return Err(PrxParseError::OutOfBounds);
    }
    let end = phdr
        .p_offset
        .checked_add(phdr.p_filesz as usize)
        .ok_or(PrxParseError::OutOfBounds)?;
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
    let end = file_off
        .checked_add(52)
        .ok_or(PrxParseError::NoModuleInfo)?;
    if end > data.len() {
        return Err(PrxParseError::NoModuleInfo);
    }
    let name_bytes = &data[file_off + 4..file_off + 32];
    let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(28);
    let raw = &name_bytes[..name_end];
    // Printable ASCII + space only; ASCII control bytes in a module
    // name would corrupt diagnostic strings downstream.
    if raw.is_empty() || !raw.iter().all(|&b| b.is_ascii_graphic() || b == b' ') {
        return Err(PrxParseError::NoModuleInfo);
    }
    let name = std::str::from_utf8(raw)
        .map_err(|_| PrxParseError::NoModuleInfo)?
        .to_owned();

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
    // Short-circuit on nid_ptr == 0 OR stub_ptr == 0; the latter
    // would otherwise resolve `v2f(0)` to the text segment's file
    // offset (when text vaddr starts at 0) and read instruction
    // bytes as stub vaddrs, binding exports to spurious in-text
    // addresses.
    if total == 0 || nid_ptr == 0 || stub_ptr == 0 {
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

            // Same hole as `read_export_entries`: a system export
            // entry with stub_table_ptr = 0 would resolve stub_foff
            // to the text segment's file offset and read instruction
            // bytes as OPD vaddrs.
            if nid_table_ptr != 0 && stub_table_ptr != 0 {
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
                        // Shipping firmware allows code = 0 (entry at
                        // start of text) but always sets toc. toc = 0
                        // is the corrupt-OPD signature; accepting it
                        // would publish an entry whose first
                        // GOT-relative load faults. `parse_real_liblv2`
                        // (code=0, toc=0x1c620) is the regression
                        // anchor for this branch.
                        if toc == 0 {
                            return Ok(None);
                        }
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

    let count = size / ELF64_RELA_SIZE;
    let mut relocs = Vec::with_capacity(count);

    for i in 0..count {
        let off = start + i * ELF64_RELA_SIZE;
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
    // 256 is comfortably above any real PRX library / module name
    // and consistent with ELF SHT_STRTAB conventions. A corrupt
    // name pointer that aims at unterminated bytes would otherwise
    // return hundreds of KB of segment content as a "name".
    const MAX_CSTRING_LEN: usize = 256;
    // Failed lookups embed the vaddr so corrupt name pointers stay
    // distinguishable from legitimately-empty strings downstream.
    let foff = match v2f(seg_map, vaddr) {
        Some(o) => o,
        None => return format!("<unmapped:0x{vaddr:x}>"),
    };
    if foff >= data.len() {
        return format!("<oob:0x{vaddr:x}>");
    }
    let scan_end = (foff + MAX_CSTRING_LEN).min(data.len());
    let end = data[foff..scan_end]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(scan_end - foff);
    String::from_utf8_lossy(&data[foff..foff + end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprx::{R_PPC64_ADDR16_HA, R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO, R_PPC64_ADDR32};

    use super::super::test_fixtures::make_test_prx;

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
}
