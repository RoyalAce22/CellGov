//! The import-table parser.
//!
//! Walks `PrxParamHeader` in PT_0x60000002 to enumerate imported
//! modules / NIDs / GOT slots. Downstream callers patch the GOT slots
//! to point at firmware-exported OPDs.

use crate::loader;
use cellgov_ps3_abi::format::elf::{
    ELF_HEADER_SIZE, ELF_PHENTSIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET,
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

/// One imported variable: VNID and the address of the slot the binder
/// patches to point at the exporter's storage.
///
/// The variable section has the same shape as the function section: an
/// id array paired with a parallel array of 4-byte slots. The
/// installed firmware witnesses that shape on its function half, where
/// `cellSysutil_Library` binds `sysPrxForUser` NIDs against
/// consecutive 4-byte GOT slots.
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
    /// An ELF64 program-header slot is smaller than its fixed layout.
    #[error("ELF64 program-header entry size {phentsize} below minimum {ELF_PHENTSIZE}")]
    BadPhentsize {
        /// Declared slot size.
        phentsize: usize,
    },
    /// An ADDR32 relocation patch offset is not 4-byte aligned.
    #[error("PRX ADDR32 relocation patch offset 0x{offset:x} is not 4-byte aligned")]
    RelocPatchMisaligned {
        /// Offset within the target segment.
        offset: u64,
    },
    /// An ADDR32 relocation value needs bits above bit 31.
    #[error("PRX ADDR32 relocation value 0x{value:x} exceeds 32 bits")]
    RelocOverflow {
        /// Computed relocation value.
        value: u64,
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
    // Read the program headers before `data` below names the relocated
    // image; a relocation inside the header's segment rewrites these words.
    let phoff = loader::read_u64(data, ELF_PHOFF_OFFSET) as usize;
    let phentsize = loader::read_u16(data, ELF_PHENTSIZE_OFFSET) as usize;
    let phnum = loader::read_u16(data, ELF_PHNUM_OFFSET) as usize;
    // Every slot reader below bounds the slot by `phentsize`, then
    // reads fields at the fixed ELF64 offsets. A slot narrower than
    // the layout puts those reads past the checked bound.
    if phnum > 0 && phentsize < ELF_PHENTSIZE {
        return Err(ImportParseError::BadPhentsize { phentsize });
    }

    // Every table pointer below is a relocation target, so the walk
    // reads the resolved image.
    let image = crate::sprx::relocated_pointer_image(data).map_err(map_reloc_error)?;
    let data: &[u8] = &image;

    let (imports_table_start_vaddr, imports_table_end_vaddr) =
        match locate_imports_via_prx_param(data, phoff, phentsize, phnum)? {
            PrxParamLocation::Table(start, end) => (start, end),
            PrxParamLocation::Empty => return Ok(Vec::new()),
            PrxParamLocation::Absent => {
                locate_imports_via_library_info(data, phoff, phentsize, phnum)?
                    .ok_or(ImportParseError::NoImportsTable)?
            }
        };

    let segments = build_segment_map(data, phoff, phentsize, phnum);

    let mut modules = Vec::new();
    let mut addr_vaddr = imports_table_start_vaddr;
    while addr_vaddr < imports_table_end_vaddr {
        let (entry, entry_end_vaddr) =
            read_import_entry(data, &segments, addr_vaddr, imports_table_end_vaddr)?;
        let name = read_cstring(data, &segments, entry.name_ptr)?;
        let functions = read_nid_stub_table(
            data,
            &segments,
            entry.nid_ptr,
            entry.stub_ptr,
            entry.function_count,
        )?
        .into_iter()
        .map(|(nid, stub_addr)| ImportedFunction { nid, stub_addr })
        .collect();
        let variables = read_nid_stub_table(
            data,
            &segments,
            entry.vnid_ptr,
            entry.vstub_ptr,
            entry.variable_count,
        )?
        .into_iter()
        .map(|(vnid, vref_addr)| ImportedVariable { vnid, vref_addr })
        .collect();

        modules.push(ImportedModule {
            name,
            functions,
            variables,
        });
        addr_vaddr = entry_end_vaddr;
    }

    Ok(modules)
}

/// The import-parse error for a relocated-image failure.
fn map_reloc_error(error: crate::sprx::RelocatedPointerError) -> ImportParseError {
    match error {
        crate::sprx::RelocatedPointerError::OutOfBounds => ImportParseError::OutOfBounds,
        crate::sprx::RelocatedPointerError::BadPhentsize { phentsize } => {
            ImportParseError::BadPhentsize { phentsize }
        }
        crate::sprx::RelocatedPointerError::RelocPatchMisaligned { offset } => {
            ImportParseError::RelocPatchMisaligned { offset }
        }
        crate::sprx::RelocatedPointerError::RelocOverflow { value } => {
            ImportParseError::RelocOverflow { value }
        }
    }
}

/// The fields of one import entry the walk reads.
///
/// An entry too small to cover `vstubs_ptr` has no variable section; its
/// variable fields read zero.
struct ImportEntryHeader {
    function_count: u16,
    name_ptr: u32,
    nid_ptr: u32,
    stub_ptr: u32,
    variable_count: u16,
    vnid_ptr: u32,
    vstub_ptr: u32,
}

/// Decodes the import entry at `addr_vaddr` and returns it with the
/// v-addr of the entry after it. The entry must end at or before
/// `table_end`.
fn read_import_entry(
    data: &[u8],
    segments: &[Segment],
    addr_vaddr: u32,
    table_end: u32,
) -> Result<(ImportEntryHeader, u32), ImportParseError> {
    let foff = vaddr_to_file(segments, addr_vaddr as usize).ok_or(ImportParseError::OutOfBounds)?;
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
    if entry_end_vaddr > table_end {
        return Err(ImportParseError::EntryPastImportsTable {
            entry_start: addr_vaddr,
            entry_size: entry_size as u32,
            imports_table_end: table_end,
        });
    }

    // Variable imports are only present when the declared entry
    // size covers the `vstubs_ptr` field. Older 28-byte (`0x1C`)
    // entries have function imports only.
    let has_variables = entry_size as u8 >= PRX_IMPORT_ENTRY_VAR_MIN_SIZE;
    let var_u16 = |offset| {
        if has_variables {
            loader::read_u16(data, foff + offset)
        } else {
            0
        }
    };
    let var_u32 = |offset| {
        if has_variables {
            loader::read_u32(data, foff + offset)
        } else {
            0
        }
    };
    let header = ImportEntryHeader {
        function_count: loader::read_u16(data, foff + PRX_IMPORT_NUM_FUNC_OFFSET),
        name_ptr: loader::read_u32(data, foff + PRX_IMPORT_NAME_PTR_OFFSET),
        nid_ptr: loader::read_u32(data, foff + PRX_IMPORT_NIDS_PTR_OFFSET),
        stub_ptr: loader::read_u32(data, foff + PRX_IMPORT_STUB_PTR_OFFSET),
        variable_count: var_u16(PRX_IMPORT_NUM_VAR_OFFSET),
        vnid_ptr: var_u32(PRX_IMPORT_VNIDS_PTR_OFFSET),
        vstub_ptr: var_u32(PRX_IMPORT_VSTUBS_PTR_OFFSET),
    };
    Ok((header, entry_end_vaddr))
}

/// Reads `count` (id, slot address) pairs from an id table at `nid_ptr`
/// and the parallel table of 4-byte slots at `stub_ptr`.
///
/// Function and variable imports share this shape. A failure in the id
/// table is `InvalidNidPtr`, one in the slot table `InvalidStubPtr`, and
/// the walk checks the id table first.
fn read_nid_stub_table(
    data: &[u8],
    segments: &[Segment],
    nid_ptr: u32,
    stub_ptr: u32,
    count: u16,
) -> Result<Vec<(u32, u32)>, ImportParseError> {
    let mut entries = Vec::with_capacity(count as usize);
    if count == 0 {
        return Ok(entries);
    }
    let bad_nid = ImportParseError::InvalidNidPtr {
        vaddr: nid_ptr,
        function_count: count,
    };
    let bad_stub = ImportParseError::InvalidStubPtr {
        vaddr: stub_ptr,
        function_count: count,
    };
    // Both tables must lie wholly inside one PT_LOAD; the binder
    // would otherwise patch across a segment boundary.
    validate_stub_ptr_range(segments, nid_ptr, count).ok_or_else(|| bad_nid.clone())?;
    validate_stub_ptr_range(segments, stub_ptr, count).ok_or_else(|| bad_stub.clone())?;

    for i in 0..count {
        // A u16 index times 4 fits u32.
        let offset = u32::from(i) * 4;
        let nid_foff = nid_ptr
            .checked_add(offset)
            .and_then(|vaddr| vaddr_to_file(segments, vaddr as usize))
            .filter(|foff| foff.checked_add(4).is_some_and(|end| end <= data.len()))
            .ok_or_else(|| bad_nid.clone())?;
        let nid = loader::read_u32(data, nid_foff);
        let stub_addr = stub_ptr
            .checked_add(offset)
            .ok_or_else(|| bad_stub.clone())?;
        entries.push((nid, stub_addr));
    }
    Ok(entries)
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

/// The declared function stub nearest an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearestStub<'a> {
    /// Name of the module that declares the stub.
    pub module: &'a str,
    /// The stub's address.
    pub stub_addr: u32,
    /// `|stub_addr - target|` in bytes.
    pub distance: u32,
}

/// The function stub among `modules` nearest `target`, or `None` when
/// they declare no function.
///
/// On a tie the first stub in module and declaration order wins.
pub fn nearest_stub<'a>(
    modules: impl IntoIterator<Item = &'a ImportedModule>,
    target: u32,
) -> Option<NearestStub<'a>> {
    let mut best: Option<NearestStub<'a>> = None;
    for m in modules {
        for f in &m.functions {
            let distance = f.stub_addr.abs_diff(target);
            if best.is_none_or(|b| distance < b.distance) {
                best = Some(NearestStub {
                    module: m.name.as_str(),
                    stub_addr: f.stub_addr,
                    distance,
                });
            }
        }
    }
    best
}

// -- Internal helpers --

/// Outcome of the `PT_PRX_PARAM` search.
enum PrxParamLocation {
    /// No LOOS+2 segment in the program header table; the caller
    /// falls back to the `ppu_prx_library_info` locator.
    Absent,
    /// A LOOS+2 segment is present but carries no body. The image
    /// declares no imports; the caller must not fall back, because
    /// segment 0's `p_paddr` is a valid load address on such images
    /// and the fallback would parse it as a library-info pointer.
    Empty,
    /// Imports table `(start, end)` v-addrs.
    Table(u32, u32),
}

/// Locate the imports table via a `PT_PRX_PARAM` (LOOS+2) program
/// header carrying a `PrxParamHeader`. Used by game ELFs and
/// user-mode PRXs.
fn locate_imports_via_prx_param(
    data: &[u8],
    phoff: usize,
    phentsize: usize,
    phnum: usize,
) -> Result<PrxParamLocation, ImportParseError> {
    let mut prx_param = None;
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
            let p_filesz = loader::read_u64(data, base + PHDR_P_FILESZ_OFFSET);
            prx_param = Some((p_offset, p_filesz));
            break;
        }
    }
    let Some((param_off, param_filesz)) = prx_param else {
        return Ok(PrxParamLocation::Absent);
    };
    // A bodyless LOOS+2 segment is the CoreOS/vsh shape: its
    // p_offset is 0, so reading the header here would decode the
    // ELF identification bytes as a PrxParamHeader.
    if param_filesz == 0 {
        return Ok(PrxParamLocation::Empty);
    }
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
    Ok(PrxParamLocation::Table(start, end_vaddr))
}

/// Locate the imports table via a `ppu_prx_library_info` struct
/// referenced from segment 0's `p_paddr` field. Returns the
/// `(start, end)` v-addrs of the table on success, or `None` when
/// either no segments are present or segment 0's `p_paddr` is zero
/// (the "no library info" signal).
///
/// This is the only locator the installed firmware exercises. No
/// module under `dev_flash/sys/external` carries a `PT_PRX_PARAM`
/// segment; every one resolves its import table through segment 0's
/// `p_paddr`. No module declares the zero sentinel, so that arm stays
/// unexercised.
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

    // Sony repurposes `p_paddr` as a file offset. The struct sits at
    // runtime address `segs[0].addr + p_paddr - p_offset`, so the
    // offset within segment 0 is `p_paddr - p_offset` and the file
    // offset is plain `p_paddr`. That holds whether or not p_vaddr
    // equals p_offset, and every installed firmware module locates its
    // import table this way. We bound only against the file end; PS3
    // segments are runtime-contiguous so `library_info` may
    // legitimately live in a later PT_LOAD than segment 0 (matches
    // what `parse_prx` accepts on the same field for `module_info`).
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
#[path = "tests/imports_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/imports_finding_tests.rs"]
mod finding_tests;
