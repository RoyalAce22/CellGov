//! The module info, the export tables and the system OPDs.

use cellgov_ps3_abi::format::elf::{EXPORT_ATTR_SYSTEM, EXPORT_ENTRY_MIN_SIZE};

use crate::loader;

use super::parse::{read_cstring, PrxExport, PrxExportLib, PrxOpd, PrxParseError};
use super::phdr::{v2f, SegEntry, VaddrRange};

/// Parse `sys_prx_module_info_t` at `file_off`.
///
/// Layout: `+0` u16 attrs, `+2` u8[2] version, `+4` char[28] name, `+32` u32
/// toc, `+36/+40` u32 exports_{start,end} (vaddr), `+44/+48` u32
/// imports_{start,end} (vaddr).
pub(super) fn parse_module_info(
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
pub(super) fn parse_export_table(
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
pub(super) fn find_system_opd(
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
