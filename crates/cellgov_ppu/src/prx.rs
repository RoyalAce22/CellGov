//! PS3 PRX import-table parser.
//!
//! Walks `PrxParamHeader` in PT_0x60000002 to enumerate imported
//! modules / NIDs / GOT slots. Downstream callers patch the GOT slots
//! to point at firmware-exported OPDs.

use crate::loader;
use cellgov_ps3_abi::elf::{
    ELF_HEADER_SIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET,
    PHDR_P_FILESZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_PADDR_OFFSET, PHDR_P_VADDR_OFFSET,
    PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_ENTRY_VAR_MIN_SIZE, PRX_IMPORT_NAME_PTR_OFFSET,
    PRX_IMPORT_NIDS_PTR_OFFSET, PRX_IMPORT_NUM_FUNC_OFFSET, PRX_IMPORT_NUM_VAR_OFFSET,
    PRX_IMPORT_SIZE_OFFSET, PRX_IMPORT_STUB_PTR_OFFSET, PRX_IMPORT_VNIDS_PTR_OFFSET,
    PRX_IMPORT_VSTUBS_PTR_OFFSET, PRX_LIB_INFO_IMPORTS_END_OFFSET,
    PRX_LIB_INFO_IMPORTS_START_OFFSET, PRX_LIB_INFO_SIZE, PRX_NAME_MAX_LEN,
    PRX_PARAM_HEADER_MIN_SIZE, PRX_PARAM_HEADER_SIZE_OFFSET, PRX_PARAM_IMPORTS_END_OFFSET,
    PRX_PARAM_IMPORTS_START_OFFSET, PRX_PARAM_MAGIC, PRX_PARAM_MAGIC_OFFSET, PT_LOAD, PT_PRX_PARAM,
};

/// A single imported PRX module with its function and variable
/// imports.
#[derive(Debug, Clone)]
pub struct ImportedModule {
    /// Module name (e.g., `cellGcmSys`).
    pub name: String,
    /// Function imports declared by this module.
    pub functions: Vec<ImportedFunction>,
    /// Variable imports declared by this module. Populated for
    /// `PrxImportEntry` records whose declared size is at least 36
    /// bytes (i.e., covers through `vstubs_ptr`); smaller entries
    /// have no variable section and produce an empty `Vec`.
    pub variables: Vec<ImportedVariable>,
}

/// One imported function: NID and the GOT slot the binder patches.
#[derive(Debug, Clone, Copy)]
pub struct ImportedFunction {
    /// Function NID (hashed name).
    pub nid: u32,
    /// Guest address of the GOT slot; the binder overwrites its
    /// contents with an OPD address so callers dereference it as a
    /// normal PPC function pointer.
    pub stub_addr: u32,
}

/// One imported variable: VNID and the address of the slot the
/// binder patches to point at the exporter's storage. Mirrors
/// RPCS3's variable-import handling in its `PPUModule.cpp`.
#[derive(Debug, Clone, Copy)]
pub struct ImportedVariable {
    /// Variable NID (hashed name).
    pub vnid: u32,
    /// Guest address of the 4-byte slot that holds the imported
    /// variable's address. The binder writes the exporter's storage
    /// address into this slot at boot.
    pub vref_addr: u32,
}

/// Failure modes for [`parse_imports`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportParseError {
    /// Neither known locator found an imports table: no
    /// `PT_PRX_PARAM` (LOOS+2) program header, and no
    /// `ppu_prx_library_info` reachable via segment 0's `p_paddr`.
    #[error(
        "no imports table: neither PT_PRX_PARAM (LOOS+2) nor \
         ppu_prx_library_info (via segment 0 p_paddr) was found"
    )]
    NoImportsTable,
    /// `PrxParamHeader` magic did not match `PRX_PARAM_MAGIC`.
    #[error("PrxParamHeader magic 0x{:08x} != expected 0x{:08x}", .0, PRX_PARAM_MAGIC)]
    BadMagic(u32),
    /// `header_size` declared smaller than the
    /// imports_table_start/end fields at +24/+28.
    #[error(
        "PrxParamHeader.header_size {} below minimum {} (imports table fields live at +24/+28)",
        .0, PRX_PARAM_HEADER_MIN_SIZE
    )]
    ParamHeaderTooSmall(u32),
    /// A read or virtual-address resolution went past the file or segment.
    #[error("read or vaddr resolution past file or segment")]
    OutOfBounds,
    /// `imports_table_end` is below `imports_table_start`; the
    /// table's bounds are inverted or the header is corrupt.
    #[error("imports_table_end 0x{end:08x} below imports_table_start 0x{start:08x}")]
    BadImportsTableRange {
        /// Declared start v-addr.
        start: u32,
        /// Declared end v-addr.
        end: u32,
    },
    /// One import entry's declared `size` byte is below
    /// [`PRX_IMPORT_ENTRY_MIN_SIZE`]; reading the entry's fields at
    /// `+16/+20/+24` would consume bytes belonging to the next entry.
    #[error(
        "import entry size byte 0x{declared:02x} below minimum 0x{:02x}",
        PRX_IMPORT_ENTRY_MIN_SIZE
    )]
    EntryTooSmall {
        /// Declared `size` byte from the entry header.
        declared: u8,
    },
    /// The current entry's bounds (`entry_start + entry_size`) extend
    /// past the declared `imports_table_end`. Either the entry is
    /// corrupt or the table's `end` field is wrong; both surface here.
    #[error(
        "import entry at 0x{entry_start:08x} ({entry_size} bytes) extends past imports_table_end 0x{imports_table_end:08x}"
    )]
    EntryPastImportsTable {
        /// Entry's start v-addr.
        entry_start: u32,
        /// Declared entry size in bytes.
        entry_size: u32,
        /// Declared `imports_table_end` v-addr.
        imports_table_end: u32,
    },
    /// An import entry's `name_ptr` did not resolve to any PT_LOAD
    /// segment, or its NUL terminator was not found within
    /// [`PRX_NAME_MAX_LEN`] bytes of the resolved file offset
    /// (or before the containing segment's end).
    #[error(
        "import name_ptr 0x{vaddr:08x} unmapped, or NUL not found within {} byte(s) or segment end",
        PRX_NAME_MAX_LEN
    )]
    InvalidNamePtr {
        /// The v-addr the entry declared.
        vaddr: u32,
    },
    /// An import entry's `stub_ptr` plus
    /// `function_count * 4` does not fit in a single PT_LOAD segment.
    /// The GOT-patch step would write into unmapped memory or across
    /// segment boundaries.
    #[error(
        "import stub_ptr 0x{vaddr:08x} unmapped, or stub table \
         ({function_count} * 4 bytes) crosses a PT_LOAD segment boundary"
    )]
    InvalidStubPtr {
        /// The v-addr the entry declared.
        vaddr: u32,
        /// Number of slots needed (`function_count`).
        function_count: u16,
    },
    /// An import entry's NID array (`nid_ptr` plus
    /// `function_count * 4`) straddles a segment boundary or extends
    /// past the file. The trailing entries would otherwise read
    /// bytes from an unrelated segment or zero-padding.
    #[error(
        "import nid_ptr 0x{vaddr:08x} unmapped, or NID table \
         ({function_count} * 4 bytes) crosses a PT_LOAD segment boundary"
    )]
    InvalidNidPtr {
        /// The v-addr the entry declared.
        vaddr: u32,
        /// Number of NIDs needed (`function_count`).
        function_count: u16,
    },
}

/// Enumerate every imported module and its (NID, GOT slot) entries.
///
/// # Errors
///
/// See [`ImportParseError`] for the typed rejection set; the parser
/// never returns a partially-populated `Vec`.
pub fn parse_imports(data: &[u8]) -> Result<Vec<ImportedModule>, ImportParseError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(ImportParseError::OutOfBounds);
    }
    let phoff = loader::read_u64(data, ELF_PHOFF_OFFSET) as usize;
    let phentsize = loader::read_u16(data, ELF_PHENTSIZE_OFFSET) as usize;
    let phnum = loader::read_u16(data, ELF_PHNUM_OFFSET) as usize;

    let (imports_table_start_vaddr, imports_table_end_vaddr) =
        match locate_imports_via_prx_param(data, phoff, phentsize, phnum)? {
            Some(table) => table,
            None => locate_imports_via_library_info(data, phoff, phentsize, phnum)?
                .ok_or(ImportParseError::NoImportsTable)?,
        };

    let segments = build_segment_map(data, phoff, phentsize, phnum);

    let mut modules = Vec::new();
    let mut addr_vaddr = imports_table_start_vaddr;
    while addr_vaddr < imports_table_end_vaddr {
        let foff =
            vaddr_to_file(&segments, addr_vaddr as usize).ok_or(ImportParseError::OutOfBounds)?;
        if foff >= data.len() {
            return Err(ImportParseError::OutOfBounds);
        }

        let entry_size_byte = data[foff + PRX_IMPORT_SIZE_OFFSET];
        if entry_size_byte < PRX_IMPORT_ENTRY_MIN_SIZE {
            return Err(ImportParseError::EntryTooSmall {
                declared: entry_size_byte,
            });
        }
        let entry_size = entry_size_byte as usize;
        let entry_end_file = foff
            .checked_add(entry_size)
            .ok_or(ImportParseError::OutOfBounds)?;
        if entry_end_file > data.len() {
            return Err(ImportParseError::OutOfBounds);
        }
        let entry_end_vaddr = addr_vaddr
            .checked_add(entry_size as u32)
            .ok_or(ImportParseError::OutOfBounds)?;
        if entry_end_vaddr > imports_table_end_vaddr {
            return Err(ImportParseError::EntryPastImportsTable {
                entry_start: addr_vaddr,
                entry_size: entry_size as u32,
                imports_table_end: imports_table_end_vaddr,
            });
        }

        let function_count = loader::read_u16(data, foff + PRX_IMPORT_NUM_FUNC_OFFSET);
        let name_ptr = loader::read_u32(data, foff + PRX_IMPORT_NAME_PTR_OFFSET);
        let nid_ptr = loader::read_u32(data, foff + PRX_IMPORT_NIDS_PTR_OFFSET);
        let stub_ptr = loader::read_u32(data, foff + PRX_IMPORT_STUB_PTR_OFFSET);

        // Variable imports are only present when the declared entry
        // size covers the `vstubs_ptr` field. Older 28-byte (`0x1C`)
        // entries have function imports only.
        let has_variables = entry_size as u8 >= PRX_IMPORT_ENTRY_VAR_MIN_SIZE;
        let variable_count = if has_variables {
            loader::read_u16(data, foff + PRX_IMPORT_NUM_VAR_OFFSET)
        } else {
            0
        };
        let vnid_ptr = if has_variables {
            loader::read_u32(data, foff + PRX_IMPORT_VNIDS_PTR_OFFSET)
        } else {
            0
        };
        let vstub_ptr = if has_variables {
            loader::read_u32(data, foff + PRX_IMPORT_VSTUBS_PTR_OFFSET)
        } else {
            0
        };

        let name = read_cstring(data, &segments, name_ptr)?;

        let mut functions = Vec::with_capacity(function_count as usize);
        if function_count > 0 {
            // Both tables must lie wholly inside one PT_LOAD; the
            // binder would otherwise patch across a segment boundary.
            validate_stub_ptr_range(&segments, nid_ptr, function_count).ok_or(
                ImportParseError::InvalidNidPtr {
                    vaddr: nid_ptr,
                    function_count,
                },
            )?;
            validate_stub_ptr_range(&segments, stub_ptr, function_count).ok_or(
                ImportParseError::InvalidStubPtr {
                    vaddr: stub_ptr,
                    function_count,
                },
            )?;

            for i in 0..function_count {
                let element_vaddr = nid_ptr
                    .checked_add(u32::from(i).checked_mul(4).ok_or(
                        ImportParseError::InvalidNidPtr {
                            vaddr: nid_ptr,
                            function_count,
                        },
                    )?)
                    .ok_or(ImportParseError::InvalidNidPtr {
                        vaddr: nid_ptr,
                        function_count,
                    })?;
                let nid_foff = vaddr_to_file(&segments, element_vaddr as usize).ok_or(
                    ImportParseError::InvalidNidPtr {
                        vaddr: nid_ptr,
                        function_count,
                    },
                )?;
                if nid_foff.checked_add(4).is_none_or(|end| end > data.len()) {
                    return Err(ImportParseError::InvalidNidPtr {
                        vaddr: nid_ptr,
                        function_count,
                    });
                }
                let nid = loader::read_u32(data, nid_foff);
                let stub_addr = stub_ptr.checked_add(u32::from(i) * 4).ok_or(
                    ImportParseError::InvalidStubPtr {
                        vaddr: stub_ptr,
                        function_count,
                    },
                )?;
                functions.push(ImportedFunction { nid, stub_addr });
            }
        }

        let mut variables = Vec::with_capacity(variable_count as usize);
        if variable_count > 0 {
            // The vnids and vstubs tables must each lie wholly inside
            // one PT_LOAD (same constraint as the function tables);
            // a slot the binder writes across a segment boundary is a
            // hard error.
            validate_stub_ptr_range(&segments, vnid_ptr, variable_count).ok_or(
                ImportParseError::InvalidNidPtr {
                    vaddr: vnid_ptr,
                    function_count: variable_count,
                },
            )?;
            validate_stub_ptr_range(&segments, vstub_ptr, variable_count).ok_or(
                ImportParseError::InvalidStubPtr {
                    vaddr: vstub_ptr,
                    function_count: variable_count,
                },
            )?;
            for i in 0..variable_count {
                let nid_vaddr = vnid_ptr.checked_add(u32::from(i) * 4).ok_or(
                    ImportParseError::InvalidNidPtr {
                        vaddr: vnid_ptr,
                        function_count: variable_count,
                    },
                )?;
                let nid_foff = vaddr_to_file(&segments, nid_vaddr as usize).ok_or(
                    ImportParseError::InvalidNidPtr {
                        vaddr: vnid_ptr,
                        function_count: variable_count,
                    },
                )?;
                if nid_foff.checked_add(4).is_none_or(|end| end > data.len()) {
                    return Err(ImportParseError::InvalidNidPtr {
                        vaddr: vnid_ptr,
                        function_count: variable_count,
                    });
                }
                let vnid = loader::read_u32(data, nid_foff);
                let vref_addr = vstub_ptr.checked_add(u32::from(i) * 4).ok_or(
                    ImportParseError::InvalidStubPtr {
                        vaddr: vstub_ptr,
                        function_count: variable_count,
                    },
                )?;
                variables.push(ImportedVariable { vnid, vref_addr });
            }
        }

        modules.push(ImportedModule {
            name,
            functions,
            variables,
        });
        addr_vaddr = entry_end_vaddr;
    }

    Ok(modules)
}

/// Human-readable one-line-per-module summary for diagnostics.
pub fn import_summary(modules: &[ImportedModule]) -> String {
    let total_funcs: usize = modules.iter().map(|m| m.functions.len()).sum();
    let mut out = format!("{} modules, {} functions:\n", modules.len(), total_funcs);
    for m in modules {
        out.push_str(&format!("  {} ({} functions)\n", m.name, m.functions.len()));
    }
    out
}

// -- Internal helpers --

/// Locate the imports table via a `PT_PRX_PARAM` (LOOS+2) program
/// header carrying a `PrxParamHeader`. Returns the `(start, end)`
/// v-addrs of the table on success, or `None` if no LOOS+2 segment
/// was found. Used by game ELFs and user-mode PRXs.
fn locate_imports_via_prx_param(
    data: &[u8],
    phoff: usize,
    phentsize: usize,
    phnum: usize,
) -> Result<Option<(u32, u32)>, ImportParseError> {
    let mut prx_param_offset = None;
    for i in 0..phnum {
        let base = phoff
            .checked_add(
                i.checked_mul(phentsize)
                    .ok_or(ImportParseError::OutOfBounds)?,
            )
            .ok_or(ImportParseError::OutOfBounds)?;
        let end = base
            .checked_add(phentsize)
            .ok_or(ImportParseError::OutOfBounds)?;
        if end > data.len() {
            return Err(ImportParseError::OutOfBounds);
        }
        let p_type = loader::read_u32(data, base);
        if p_type == PT_PRX_PARAM {
            let p_offset = loader::read_u64(data, base + PHDR_P_OFFSET_OFFSET) as usize;
            prx_param_offset = Some(p_offset);
            break;
        }
    }
    let Some(param_off) = prx_param_offset else {
        return Ok(None);
    };
    let param_end = param_off
        .checked_add(PRX_PARAM_HEADER_MIN_SIZE as usize)
        .ok_or(ImportParseError::OutOfBounds)?;
    if param_end > data.len() {
        return Err(ImportParseError::OutOfBounds);
    }

    let magic = loader::read_u32(data, param_off + PRX_PARAM_MAGIC_OFFSET);
    if magic != PRX_PARAM_MAGIC {
        return Err(ImportParseError::BadMagic(magic));
    }
    let declared_size = loader::read_u32(data, param_off + PRX_PARAM_HEADER_SIZE_OFFSET);
    if declared_size < PRX_PARAM_HEADER_MIN_SIZE {
        return Err(ImportParseError::ParamHeaderTooSmall(declared_size));
    }

    let start = loader::read_u32(data, param_off + PRX_PARAM_IMPORTS_START_OFFSET);
    let end_vaddr = loader::read_u32(data, param_off + PRX_PARAM_IMPORTS_END_OFFSET);
    if end_vaddr < start {
        return Err(ImportParseError::BadImportsTableRange {
            start,
            end: end_vaddr,
        });
    }
    Ok(Some((start, end_vaddr)))
}

/// Locate the imports table via a `ppu_prx_library_info` struct
/// referenced from segment 0's `p_paddr` field. Returns the
/// `(start, end)` v-addrs of the table on success, or `None` when
/// either no segments are present or segment 0's `p_paddr` is zero
/// (the "no library info" signal). Matches RPCS3's firmware-PRX
/// path in `PPUModule.cpp`.
fn locate_imports_via_library_info(
    data: &[u8],
    phoff: usize,
    phentsize: usize,
    phnum: usize,
) -> Result<Option<(u32, u32)>, ImportParseError> {
    if phnum == 0 {
        return Ok(None);
    }
    let phdr0_end = phoff
        .checked_add(phentsize)
        .ok_or(ImportParseError::OutOfBounds)?;
    if phdr0_end > data.len() {
        return Err(ImportParseError::OutOfBounds);
    }
    // PS3 SPRX layout invariant: segment 0 is the text PT_LOAD that
    // carries the library_info struct.
    let p_paddr = loader::read_u64(data, phoff + PHDR_P_PADDR_OFFSET);
    if p_paddr == 0 {
        return Ok(None);
    }

    // RPCS3 reads the struct at runtime address
    // `segs[0].addr + p_paddr - p_offset`. `segs[0].addr` is the
    // load-base of segment 0, so the offset-within-segment is
    // `p_paddr - p_offset` and the file offset is `p_paddr`. This
    // interprets p_paddr as Sony-repurposed-file-offset, matching
    // the formula whether or not p_vaddr equals p_offset. We bound
    // only against the file end; PS3 segments are runtime-
    // contiguous so `library_info` may legitimately live in a
    // later PT_LOAD than segment 0 (matches what `parse_prx`
    // accepts on the same field for `module_info`).
    let lib_info_foff = usize::try_from(p_paddr).map_err(|_| ImportParseError::OutOfBounds)?;
    if lib_info_foff
        .checked_add(PRX_LIB_INFO_SIZE)
        .is_none_or(|end| end > data.len())
    {
        return Err(ImportParseError::OutOfBounds);
    }
    let start = loader::read_u32(data, lib_info_foff + PRX_LIB_INFO_IMPORTS_START_OFFSET);
    let end = loader::read_u32(data, lib_info_foff + PRX_LIB_INFO_IMPORTS_END_OFFSET);
    if end < start {
        return Err(ImportParseError::BadImportsTableRange { start, end });
    }
    Ok(Some((start, end)))
}

struct Segment {
    vaddr: usize,
    file_offset: usize,
    size: usize,
}

fn build_segment_map(data: &[u8], phoff: usize, phentsize: usize, phnum: usize) -> Vec<Segment> {
    let mut segs = Vec::new();
    for i in 0..phnum {
        let base = match phoff.checked_add(i.saturating_mul(phentsize)) {
            Some(v) => v,
            None => break,
        };
        let end = match base.checked_add(phentsize) {
            Some(v) => v,
            None => break,
        };
        if end > data.len() {
            break;
        }
        let p_type = loader::read_u32(data, base);
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = loader::read_u64(data, base + PHDR_P_OFFSET_OFFSET) as usize;
        let p_vaddr = loader::read_u64(data, base + PHDR_P_VADDR_OFFSET) as usize;
        let p_filesz = loader::read_u64(data, base + PHDR_P_FILESZ_OFFSET) as usize;
        if p_filesz > 0 {
            segs.push(Segment {
                vaddr: p_vaddr,
                file_offset: p_offset,
                size: p_filesz,
            });
        }
    }
    segs
}

fn vaddr_to_file(segments: &[Segment], vaddr: usize) -> Option<usize> {
    for seg in segments {
        let end = seg.vaddr.checked_add(seg.size)?;
        if vaddr >= seg.vaddr && vaddr < end {
            let delta = vaddr - seg.vaddr;
            return seg.file_offset.checked_add(delta);
        }
    }
    None
}

/// Locate the PT_LOAD segment containing `vaddr` and return both its
/// file offset for `vaddr` and the byte distance to the end of the
/// containing segment. Used by readers that need to bound their walk
/// to a single PT_LOAD without straddling into adjacent segments.
fn vaddr_to_file_with_segment_remainder(
    segments: &[Segment],
    vaddr: usize,
) -> Option<(usize, usize)> {
    for seg in segments {
        let seg_end = seg.vaddr.checked_add(seg.size)?;
        if vaddr >= seg.vaddr && vaddr < seg_end {
            let delta = vaddr - seg.vaddr;
            let foff = seg.file_offset.checked_add(delta)?;
            let remainder = seg.size - delta;
            return Some((foff, remainder));
        }
    }
    None
}

/// Verify a contiguous span of `function_count * 4` u32 slots
/// starting at `base_vaddr` lies entirely inside one PT_LOAD segment.
/// Returns `Some((file_offset, byte_length))` on success, `None` if
/// the base is unmapped, the span exceeds u32 / usize arithmetic, or
/// the span would cross a segment boundary.
fn validate_stub_ptr_range(
    segments: &[Segment],
    base_vaddr: u32,
    function_count: u16,
) -> Option<(usize, usize)> {
    let byte_len = (u32::from(function_count)).checked_mul(4)?;
    let last_byte = base_vaddr.checked_add(byte_len.checked_sub(1)?)?;
    let (foff, remainder) = vaddr_to_file_with_segment_remainder(segments, base_vaddr as usize)?;
    let last_byte_in_seg = (last_byte as usize)
        .checked_sub(base_vaddr as usize)
        .is_some_and(|delta| delta < remainder);
    if !last_byte_in_seg {
        return None;
    }
    Some((foff, byte_len as usize))
}

fn read_cstring(data: &[u8], segments: &[Segment], vaddr: u32) -> Result<String, ImportParseError> {
    let (foff, remainder) = vaddr_to_file_with_segment_remainder(segments, vaddr as usize)
        .ok_or(ImportParseError::InvalidNamePtr { vaddr })?;
    if foff >= data.len() {
        return Err(ImportParseError::InvalidNamePtr { vaddr });
    }
    // Cap the walk at the smaller of: the containing segment's
    // remaining bytes, the file's remaining bytes, and the hard
    // PRX_NAME_MAX_LEN. Names longer than the cap are rejected; a
    // missing NUL within bounds is also rejected.
    let max_walk = remainder.min(data.len() - foff).min(PRX_NAME_MAX_LEN);
    let window = &data[foff..foff + max_walk];
    let nul_pos = window
        .iter()
        .position(|&b| b == 0)
        .ok_or(ImportParseError::InvalidNamePtr { vaddr })?;
    Ok(String::from_utf8_lossy(&window[..nul_pos]).into_owned())
}

#[cfg(test)]
#[path = "tests/prx_tests.rs"]
mod tests;
