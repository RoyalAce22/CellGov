//! PS3 PRX import-table parser and HLE trampoline binder.
//!
//! Walks `PrxParamHeader` in PT_0x60000002 to enumerate imported
//! modules / NIDs / GOT slots, then writes HLE trampolines into guest
//! memory and patches each GOT slot to point at its OPD.

use crate::loader;
use cellgov_mem::{ByteRange, GuestAddr, StagedWrite, StagingMemory};
use cellgov_ps3_abi::elf::{
    ELF_HEADER_SIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET,
    PHDR_P_FILESZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_PADDR_OFFSET, PHDR_P_VADDR_OFFSET,
    PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_NAME_PTR_OFFSET, PRX_IMPORT_NIDS_PTR_OFFSET,
    PRX_IMPORT_NUM_FUNC_OFFSET, PRX_IMPORT_SIZE_OFFSET, PRX_IMPORT_STUB_PTR_OFFSET,
    PRX_LIB_INFO_IMPORTS_END_OFFSET, PRX_LIB_INFO_IMPORTS_START_OFFSET, PRX_LIB_INFO_SIZE,
    PRX_NAME_MAX_LEN, PRX_PARAM_HEADER_MIN_SIZE, PRX_PARAM_HEADER_SIZE_OFFSET,
    PRX_PARAM_IMPORTS_END_OFFSET, PRX_PARAM_IMPORTS_START_OFFSET, PRX_PARAM_MAGIC,
    PRX_PARAM_MAGIC_OFFSET, PT_LOAD, PT_PRX_PARAM,
};

/// A single imported PRX module with its function imports.
#[derive(Debug, Clone)]
pub struct ImportedModule {
    /// Module name (e.g., `cellGcmSys`).
    pub name: String,
    /// Function imports declared by this module.
    pub functions: Vec<ImportedFunction>,
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

/// Failure modes for [`parse_imports`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportParseError {
    /// Neither known locator found an imports table: no
    /// `PT_PRX_PARAM` (LOOS+2) program header, and no
    /// `ppu_prx_library_info` reachable via segment 0's `p_paddr`.
    NoImportsTable,
    /// `PrxParamHeader` magic did not match `PRX_PARAM_MAGIC`.
    BadMagic(u32),
    /// `header_size` declared smaller than the
    /// imports_table_start/end fields at +24/+28.
    ParamHeaderTooSmall(u32),
    /// A read or virtual-address resolution went past the file or segment.
    OutOfBounds,
    /// `imports_table_end` is below `imports_table_start`; the
    /// table's bounds are inverted or the header is corrupt.
    BadImportsTableRange {
        /// Declared start v-addr.
        start: u32,
        /// Declared end v-addr.
        end: u32,
    },
    /// One import entry's declared `size` byte is below
    /// [`PRX_IMPORT_ENTRY_MIN_SIZE`]; reading the entry's fields at
    /// `+16/+20/+24` would consume bytes belonging to the next entry.
    EntryTooSmall {
        /// Declared `size` byte from the entry header.
        declared: u8,
    },
    /// The current entry's bounds (`entry_start + entry_size`) extend
    /// past the declared `imports_table_end`. Either the entry is
    /// corrupt or the table's `end` field is wrong; both surface here.
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
    InvalidNamePtr {
        /// The v-addr the entry declared.
        vaddr: u32,
    },
    /// An import entry's `stub_ptr` plus
    /// `function_count * 4` does not fit in a single PT_LOAD segment.
    /// The GOT-patch step would write into unmapped memory or across
    /// segment boundaries.
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
    InvalidNidPtr {
        /// The v-addr the entry declared.
        vaddr: u32,
        /// Number of NIDs needed (`function_count`).
        function_count: u16,
    },
}

impl std::fmt::Display for ImportParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoImportsTable => f.write_str(
                "no imports table: neither PT_PRX_PARAM (LOOS+2) nor \
                 ppu_prx_library_info (via segment 0 p_paddr) was found",
            ),
            Self::BadMagic(got) => write!(
                f,
                "PrxParamHeader magic 0x{got:08x} != expected 0x{:08x}",
                PRX_PARAM_MAGIC
            ),
            Self::ParamHeaderTooSmall(declared) => write!(
                f,
                "PrxParamHeader.header_size {declared} below minimum {} (imports table fields live at +24/+28)",
                PRX_PARAM_HEADER_MIN_SIZE
            ),
            Self::OutOfBounds => f.write_str("read or vaddr resolution past file or segment"),
            Self::BadImportsTableRange { start, end } => write!(
                f,
                "imports_table_end 0x{end:08x} below imports_table_start 0x{start:08x}"
            ),
            Self::EntryTooSmall { declared } => write!(
                f,
                "import entry size byte 0x{declared:02x} below minimum 0x{:02x}",
                PRX_IMPORT_ENTRY_MIN_SIZE
            ),
            Self::EntryPastImportsTable {
                entry_start,
                entry_size,
                imports_table_end,
            } => write!(
                f,
                "import entry at 0x{entry_start:08x} ({entry_size} bytes) extends past imports_table_end 0x{imports_table_end:08x}"
            ),
            Self::InvalidNamePtr { vaddr } => write!(
                f,
                "import name_ptr 0x{vaddr:08x} unmapped, or NUL not found within {} byte(s) or segment end",
                PRX_NAME_MAX_LEN
            ),
            Self::InvalidStubPtr {
                vaddr,
                function_count,
            } => write!(
                f,
                "import stub_ptr 0x{vaddr:08x} unmapped, or stub table \
                 ({function_count} * 4 bytes) crosses a PT_LOAD segment boundary"
            ),
            Self::InvalidNidPtr {
                vaddr,
                function_count,
            } => write!(
                f,
                "import nid_ptr 0x{vaddr:08x} unmapped, or NID table \
                 ({function_count} * 4 bytes) crosses a PT_LOAD segment boundary"
            ),
        }
    }
}

impl std::error::Error for ImportParseError {}

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
            // RPCS3 falls back to `sizeof(...)` on a zero-size entry
            // and keeps walking; CellGov rejects so corruption can't
            // shift the cursor mid-table.
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

        modules.push(ImportedModule { name, functions });
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

/// Lower bound of [`SyscallNamespace::HleImport`][ns]; HLE syscall
/// number for binding `i` is `HLE_SYSCALL_BASE + i`.
///
/// [ns]: cellgov_ps3_abi::syscall_namespace::SyscallNamespace::HleImport
pub const HLE_SYSCALL_BASE: u32 = cellgov_ps3_abi::syscall_namespace::SyscallNamespace::HleImport
    .range()
    .0 as u32;

/// NIDs for which CellGov ships a dedicated HLE implementation.
///
/// Consumed at the CLI layer, not by this crate's binder:
///
/// 1. `apps/cellgov_cli`'s `dump-imports` tags each game import
///    `impl` vs `stub` against this list.
/// 2. `apps/cellgov_cli`'s firmware-PRX patch step uses it as a
///    keep-list: a game import that names a listed NID stays on its
///    HLE trampoline even when a firmware module also exports it,
///    because the firmware-side `module_start` may not have
///    initialized the state the export expects.
pub const HLE_IMPLEMENTED_NIDS: &[u32] = {
    use cellgov_ps3_abi::nid::{
        cell_gcm_sys as gcm, cell_save_data as savedata, cell_spurs as spurs,
        cell_sysutil as sysutil, sys_fs as fs, sys_prx_for_user as sys,
    };
    &[
        // cellGcmSys.
        gcm::GET_TILED_PITCH_SIZE,
        gcm::INIT_BODY,
        gcm::GET_CONTROL_REGISTER,
        gcm::GET_CONFIGURATION,
        gcm::GET_LABEL_ADDRESS,
        gcm::ADDRESS_TO_OFFSET,
        // sysPrxForUser memory + process.
        sys::INITIALIZE_TLS,
        sys::MALLOC,
        sys::FREE,
        sys::MEMSET,
        sys::PROCESS_EXIT,
        sys::HEAP_CREATE_HEAP,
        // sysPrxForUser lwmutex (create + 4 stubs).
        sys::LWMUTEX_CREATE,
        sys::LWMUTEX_LOCK,
        sys::LWMUTEX_DESTROY,
        sys::LWMUTEX_UNLOCK,
        sys::LWMUTEX_TRYLOCK,
        // sysPrxForUser lwcond create/destroy only; wait/signal still
        // route through the unclaimed-NID handler.
        sys::LWCOND_CREATE,
        sys::LWCOND_DESTROY,
        // sysPrxForUser time / thread / process / prx.
        sys::TIME_GET_SYSTEM_TIME,
        sys::PPU_THREAD_GET_ID,
        sys::PPU_THREAD_CREATE,
        sys::PROCESS_IS_STACK,
        sys::PRX_EXITSPAWN_WITH_LEVEL,
        // cellSysutil video-out queries.
        sysutil::VIDEO_OUT_GET_STATE,
        sysutil::VIDEO_OUT_GET_RESOLUTION,
        // cellSpurs initialize family.
        spurs::ATTRIBUTE_INITIALIZE,
        spurs::INITIALIZE,
        spurs::INITIALIZE_WITH_ATTRIBUTE,
        spurs::INITIALIZE_WITH_ATTRIBUTE2,
        spurs::FINALIZE,
        // cellSpurs workload registry.
        spurs::WORKLOAD_ATTRIBUTE_INITIALIZE,
        spurs::ADD_WORKLOAD,
        spurs::ADD_WORKLOAD_WITH_ATTRIBUTE,
        spurs::SHUTDOWN_WORKLOAD,
        spurs::WAIT_FOR_WORKLOAD_SHUTDOWN,
        // cellSpurs ready-count, contention, idle-spu, priority controls.
        spurs::READY_COUNT_STORE,
        spurs::READY_COUNT_ADD,
        spurs::READY_COUNT_SWAP,
        spurs::READY_COUNT_COMPARE_AND_SWAP,
        spurs::REQUEST_IDLE_SPU,
        spurs::SET_MAX_CONTENTION,
        spurs::SET_PRIORITIES,
        spurs::SET_PRIORITY,
        // cellSpurs info getter + exception handler registration.
        spurs::GET_INFO,
        spurs::ATTACH_LV2_EVENT_QUEUE,
        spurs::DETACH_LV2_EVENT_QUEUE,
        spurs::SET_EXCEPTION_EVENT_HANDLER,
        spurs::UNSET_EXCEPTION_EVENT_HANDLER,
        spurs::SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
        spurs::UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER,
        spurs::ENABLE_EXCEPTION_EVENT_HANDLER,
        // sys_fs HLE wrappers; each forwards to the matching LV2
        // sys_fs_* syscall handler.
        fs::OPEN,
        fs::READ,
        fs::CLOSE,
        fs::LSEEK,
        fs::FSTAT,
        fs::STAT,
        // cellSaveData AutoLoad / AutoLoad2 only. AutoSave and
        // ListAutoLoad stay unclaimed.
        savedata::AUTO_LOAD,
        savedata::AUTO_LOAD_2,
    ]
};

/// One bound HLE import: index, originating module, NID, and GOT slot.
#[derive(Debug, Clone)]
pub struct HleBinding {
    /// 0-based; the syscall number is `HLE_SYSCALL_BASE + index`.
    pub index: u32,
    /// Name of the module this binding originated from.
    pub module: String,
    /// Function NID being bound.
    pub nid: u32,
    /// Guest address of the patched GOT slot.
    pub stub_addr: u32,
}

/// Per-binding trampoline footprint for [`HleLayout::Legacy24`]
/// (8-byte OPD + 16-byte body). `Ps3Spec` splits OPD and body.
pub const TRAMPOLINE_SIZE: u32 = 24;

/// Memory layout for HLE OPDs and trampoline bodies.
#[derive(Debug, Clone, Copy)]
pub enum HleLayout {
    /// 24 bytes per binding at `trampoline_base`: 8-byte OPD followed
    /// by an inline 16-byte `lis/ori/sc/blr` body.
    Legacy24,
    /// 8-byte OPD at `opd_base + i*8`, 16-byte body at
    /// `body_base + i*16`. Matches RPCS3's `vm::alloc(N*8, vm::main)`
    /// HLE table shape so GOT entries are packed 8-byte pointers.
    Ps3Spec {
        /// Base guest address for packed 8-byte OPDs.
        opd_base: u32,
        /// Base guest address for 16-byte trampoline bodies.
        body_base: u32,
    },
}

impl HleLayout {
    /// One-past-the-end guest address occupied by `n_bindings` worth
    /// of OPDs + bodies under this layout. For `Legacy24` the result
    /// is relative to `trampoline_base` and is computed assuming OPDs
    /// and bodies are interleaved as `24 * n_bindings` from the same
    /// base. For `Ps3Spec` the result is the absolute address.
    ///
    /// Byte-precise: the caller owns any page-alignment rounding the
    /// downstream consumer requires (e.g., 64K alignment for PRX
    /// placement on the `main` region).
    ///
    /// Returns `None` if the arithmetic overflows `u32`.
    pub fn extent_end(self, legacy_base: u32, n_bindings: u32) -> Option<u32> {
        match self {
            HleLayout::Legacy24 => legacy_base.checked_add(n_bindings.checked_mul(24)?),
            HleLayout::Ps3Spec {
                opd_base,
                body_base,
            } => {
                let opd_end = opd_base.checked_add(n_bindings.checked_mul(8)?)?;
                let body_end = body_base.checked_add(n_bindings.checked_mul(16)?)?;
                Some(opd_end.max(body_end))
            }
        }
    }
}

/// Failure surface for [`bind_hle_stubs_with_layout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindError {
    /// The total trampoline footprint (`module_count * function_count * 24`
    /// bytes for `Legacy24`, or `opd_base + N*8` / `body_base + N*16`
    /// for `Ps3Spec`) would overflow `u32`. Carries the binding index
    /// at which the overflow would happen.
    LayoutOverflow {
        /// Index of the binding whose computed address overflowed.
        binding_index: u32,
        /// Tag identifying which arithmetic overflowed, of the form
        /// `<region>_<phase>` (`legacy` / `opd` / `body` crossed with
        /// `base` / `offset` / `body_offset` / `footprint`).
        kind: &'static str,
    },
    /// The HLE binding count exceeded the capacity of the
    /// `SyscallNamespace::HleImport` range. Carries the upper bound
    /// of the namespace so a widening fix has a target value.
    NamespaceExhausted {
        /// Index at which the namespace ran out.
        binding_index: u32,
        /// Upper bound of `SyscallNamespace::HleImport.range().1`.
        namespace_upper_bound: u64,
    },
    /// A staged OPD / body / GOT write produced an unconstructible
    /// `ByteRange` (e.g. `addr + length` overflowed `u64`). Carries
    /// the slot kind so the caller can attribute the failure.
    BadRange {
        /// Slot identifier: `"opd"`, `"body"`, or `"got"`.
        kind: &'static str,
        /// Guest address that failed to round-trip into `ByteRange`.
        addr: u32,
        /// Declared length in bytes.
        length: u32,
    },
    /// `StagingMemory::drain_into` rejected the batch. Atomic
    /// guarantee: guest memory is unchanged.
    StagingCommitFailed {
        /// Underlying `cellgov_mem` error from the drain step.
        source: cellgov_mem::MemError,
        /// Number of staged writes the batch carried.
        staged_count: usize,
    },
}

/// [`bind_hle_stubs_with_layout`] with [`HleLayout::Legacy24`].
///
/// # Errors
///
/// See [`bind_hle_stubs_with_layout`] for the typed failure surface.
pub fn bind_hle_stubs(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    trampoline_base: u32,
) -> Result<Vec<HleBinding>, BindError> {
    bind_hle_stubs_with_layout(modules, memory, HleLayout::Legacy24, trampoline_base)
}

/// Stage HLE OPDs, trampoline bodies, and GOT patches for every
/// imported function and commit them as one atomic
/// [`cellgov_mem::StagingMemory`] batch.
///
/// # Atomicity
///
/// Every write the binder produces (3 per binding: OPD, body, GOT
/// slot) drains through one `StagingMemory`. On a rejected drain
/// guest memory is unchanged. Callers must not interleave other
/// guest writes through this `GuestMemory` between bindings.
///
/// # Errors
///
/// - [`BindError::LayoutOverflow`] -- a slot address (or its
///   footprint end) overflows `u32`.
/// - [`BindError::NamespaceExhausted`] -- binding count exceeds
///   `SyscallNamespace::HleImport`'s upper bound.
/// - [`BindError::BadRange`] -- a computed `ByteRange` failed to
///   round-trip into `u64`.
/// - [`BindError::StagingCommitFailed`] -- `drain_into` rejected the
///   batch.
pub fn bind_hle_stubs_with_layout(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    layout: HleLayout,
    legacy_base: u32,
) -> Result<Vec<HleBinding>, BindError> {
    use cellgov_ps3_abi::syscall_namespace::SyscallNamespace;
    use cellgov_ps3_abi::trampoline_codegen::{
        encode_blr, encode_lis_ori_sc, encode_ps3_packed_opd,
    };

    let total_bindings: usize = modules.iter().map(|m| m.functions.len()).sum();
    let mut bindings = Vec::with_capacity(total_bindings);
    let mut pending: Vec<StagedWrite> = Vec::with_capacity(total_bindings * 3);

    for module in modules {
        for func in &module.functions {
            let hle_index = bindings.len() as u32;
            let syscall_nr_u64 = SyscallNamespace::HleImport.try_encode(hle_index).ok_or(
                BindError::NamespaceExhausted {
                    binding_index: hle_index,
                    namespace_upper_bound: SyscallNamespace::HleImport.range().1,
                },
            )?;
            let syscall_nr =
                u32::try_from(syscall_nr_u64).expect("HleImport namespace fits in u32");

            let (opd_addr, body_addr) = match layout {
                HleLayout::Legacy24 => {
                    let tramp =
                        legacy_base.checked_add(hle_index.checked_mul(TRAMPOLINE_SIZE).ok_or(
                            BindError::LayoutOverflow {
                                binding_index: hle_index,
                                kind: "legacy_offset",
                            },
                        )?);
                    let tramp = tramp.ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "legacy_base",
                    })?;
                    tramp
                        .checked_add(TRAMPOLINE_SIZE)
                        .ok_or(BindError::LayoutOverflow {
                            binding_index: hle_index,
                            kind: "legacy_footprint",
                        })?;
                    let body = tramp.checked_add(8).ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "legacy_body_offset",
                    })?;
                    (tramp, body)
                }
                HleLayout::Ps3Spec {
                    opd_base,
                    body_base,
                } => {
                    let opd = opd_base.checked_add(hle_index.checked_mul(8).ok_or(
                        BindError::LayoutOverflow {
                            binding_index: hle_index,
                            kind: "opd_base",
                        },
                    )?);
                    let opd = opd.ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "opd_base",
                    })?;
                    opd.checked_add(8).ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "opd_footprint",
                    })?;
                    let body = body_base.checked_add(hle_index.checked_mul(16).ok_or(
                        BindError::LayoutOverflow {
                            binding_index: hle_index,
                            kind: "body_base",
                        },
                    )?);
                    let body = body.ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "body_base",
                    })?;
                    body.checked_add(16).ok_or(BindError::LayoutOverflow {
                        binding_index: hle_index,
                        kind: "body_footprint",
                    })?;
                    (opd, body)
                }
            };

            // OPD: { body_addr, toc=0 } in RPCS3-packed 8-byte shape.
            let opd_range =
                ByteRange::new(GuestAddr::new(opd_addr as u64), 8).ok_or(BindError::BadRange {
                    kind: "opd",
                    addr: opd_addr,
                    length: 8,
                })?;
            let opd_bytes = encode_ps3_packed_opd(body_addr, 0);
            pending.push(StagedWrite {
                range: opd_range,
                bytes: opd_bytes.to_vec(),
            });

            // Body: lis r11, hi; ori r11, r11, lo; sc 0; blr.
            let body_range = ByteRange::new(GuestAddr::new(body_addr as u64), 16).ok_or(
                BindError::BadRange {
                    kind: "body",
                    addr: body_addr,
                    length: 16,
                },
            )?;
            let lis_ori_sc = encode_lis_ori_sc(syscall_nr);
            let blr = encode_blr();
            let mut body_bytes = [0u8; 16];
            body_bytes[0..12].copy_from_slice(&lis_ori_sc);
            body_bytes[12..16].copy_from_slice(&blr);
            pending.push(StagedWrite {
                range: body_range,
                bytes: body_bytes.to_vec(),
            });

            // GOT slot: 4 bytes BE pointing at the OPD.
            let got_range = ByteRange::new(GuestAddr::new(func.stub_addr as u64), 4).ok_or(
                BindError::BadRange {
                    kind: "got",
                    addr: func.stub_addr,
                    length: 4,
                },
            )?;
            pending.push(StagedWrite {
                range: got_range,
                bytes: opd_addr.to_be_bytes().to_vec(),
            });

            bindings.push(HleBinding {
                index: hle_index,
                module: module.name.clone(),
                nid: func.nid,
                stub_addr: func.stub_addr,
            });
        }
    }

    let staged_count = pending.len();
    let mut staging = StagingMemory::new();
    for write in pending {
        staging.stage(write);
    }
    // Clear the buffer on Err so the "leaked staged writes"
    // debug-assert does not fire on the error path.
    match staging.drain_into(memory) {
        Ok(_) => Ok(bindings),
        Err(source) => {
            staging.clear();
            Err(BindError::StagingCommitFailed {
                source,
                staged_count,
            })
        }
    }
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
/// path in `PPUModule.cpp:1836-1860`.
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
mod tests {
    use super::*;

    #[test]
    fn hle_implemented_nids_is_nonempty_and_unique() {
        assert!(!HLE_IMPLEMENTED_NIDS.is_empty());
        let mut sorted = HLE_IMPLEMENTED_NIDS.to_vec();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(
            sorted.len(),
            deduped.len(),
            "HLE_IMPLEMENTED_NIDS contains a duplicate"
        );
    }

    #[test]
    fn hle_implemented_nids_contains_tls_init() {
        assert!(
            HLE_IMPLEMENTED_NIDS.contains(&cellgov_ps3_abi::nid::sys_prx_for_user::INITIALIZE_TLS)
        );
    }

    #[test]
    fn import_parse_error_display_renders_every_variant() {
        let cases: &[(ImportParseError, &[&str])] = &[
            (
                ImportParseError::NoImportsTable,
                &["PT_PRX_PARAM", "ppu_prx_library_info"],
            ),
            (
                ImportParseError::BadMagic(0xdead_beef),
                &["magic", "0xdeadbeef"],
            ),
            (
                ImportParseError::ParamHeaderTooSmall(8),
                &["header_size", "8"],
            ),
            (ImportParseError::OutOfBounds, &["past", "segment"]),
            (
                ImportParseError::BadImportsTableRange {
                    start: 0x100,
                    end: 0x80,
                },
                &["imports_table_end", "imports_table_start"],
            ),
            (
                ImportParseError::EntryTooSmall { declared: 0x10 },
                &["entry size byte", "0x10"],
            ),
            (
                ImportParseError::EntryPastImportsTable {
                    entry_start: 0xd0,
                    entry_size: 0x2c,
                    imports_table_end: 0xe0,
                },
                &["0x000000d0", "0x000000e0", "44"],
            ),
            (
                ImportParseError::InvalidNamePtr { vaddr: 0x1234 },
                &["name_ptr", "0x00001234"],
            ),
            (
                ImportParseError::InvalidStubPtr {
                    vaddr: 0x900,
                    function_count: 5,
                },
                &["stub_ptr", "0x00000900", "5", "unmapped"],
            ),
            (
                ImportParseError::InvalidNidPtr {
                    vaddr: 0x500,
                    function_count: 3,
                },
                &["nid_ptr", "0x00000500", "3", "unmapped"],
            ),
        ];
        for (err, needles) in cases {
            let s = format!("{err}");
            assert!(!s.is_empty(), "empty Display for {err:?}");
            for needle in *needles {
                assert!(
                    s.contains(needle),
                    "Display of {err:?} missing {needle:?}: {s}"
                );
            }
        }
    }

    /// Retail-fixture tests require `tools/rpcs3/dev_hdd0/...` to be
    /// populated locally. `#[ignore]` keeps `cargo test` clean on
    /// hosts without the fixtures, and the env-gated runner promotes
    /// the absent fixture into a hard failure when set
    /// (`CELLGOV_RETAIL_FIXTURES=1 cargo test -- --ignored`).
    #[test]
    #[ignore = "requires tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf; \
                run with CELLGOV_RETAIL_FIXTURES=1 cargo test -- --ignored"]
    fn parse_retail_eboot_imports() {
        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
            if std::env::var_os("CELLGOV_RETAIL_FIXTURES").is_some() {
                panic!(
                    "CELLGOV_RETAIL_FIXTURES set but {} is absent",
                    path.display()
                );
            }
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let modules = parse_imports(&data).unwrap();

        assert!(!modules.is_empty(), "should find imported modules");

        let total_funcs: usize = modules.iter().map(|m| m.functions.len()).sum();
        assert_eq!(modules.len(), 12);
        assert_eq!(total_funcs, 140);

        let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"cellSysutil"));
        assert!(names.contains(&"sysPrxForUser"));
        assert!(names.contains(&"cellGcmSys"));
    }

    /// Minimal ELF with PT_LOAD mapped 1:1 (vaddr == file offset) and
    /// one import module of one function. The GOT slot at
    /// `STUB_TABLE_OFF` is the `stub_addr` the parser reports.
    fn build_synthetic_prx_elf(nid: u32) -> Vec<u8> {
        const TOTAL_SIZE: usize = 320;
        const PARAM_OFF: usize = 176;
        const MOD_INFO_OFF: usize = 208;
        const MOD_INFO_SIZE: u8 = 0x2C;
        const NAME_OFF: usize = 252;
        const NID_TABLE_OFF: usize = 256;
        const STUB_TABLE_OFF: usize = 260;

        let mut data = vec![0u8; TOTAL_SIZE];
        // ELF header: magic, 64-bit, big-endian, phoff=64,
        // phentsize=56, phnum=2.
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&2u16.to_be_bytes());

        // PT_LOAD covering the whole file with vaddr == file offset.
        let ph0 = 64usize;
        data[ph0..ph0 + 4].copy_from_slice(&1u32.to_be_bytes());
        data[ph0 + 8..ph0 + 16].copy_from_slice(&0u64.to_be_bytes());
        data[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
        data[ph0 + 32..ph0 + 40].copy_from_slice(&(TOTAL_SIZE as u64).to_be_bytes());

        // PT_PRX_PARAM pointing at PARAM_OFF.
        let ph1 = 64 + 56;
        data[ph1..ph1 + 4].copy_from_slice(&PT_PRX_PARAM.to_be_bytes());
        data[ph1 + 8..ph1 + 16].copy_from_slice(&(PARAM_OFF as u64).to_be_bytes());

        // PrxParamHeader: header_size=0x40, magic, imports table.
        data[PARAM_OFF..PARAM_OFF + 4].copy_from_slice(&0x40u32.to_be_bytes());
        data[PARAM_OFF + 4..PARAM_OFF + 8].copy_from_slice(&PRX_PARAM_MAGIC.to_be_bytes());
        data[PARAM_OFF + 24..PARAM_OFF + 28].copy_from_slice(&(MOD_INFO_OFF as u32).to_be_bytes());
        data[PARAM_OFF + 28..PARAM_OFF + 32]
            .copy_from_slice(&(MOD_INFO_OFF as u32 + MOD_INFO_SIZE as u32).to_be_bytes());

        // PrxImportEntry: entry_size=0x2C, function_count=1,
        // name/nids/stubs ptrs.
        data[MOD_INFO_OFF] = MOD_INFO_SIZE;
        data[MOD_INFO_OFF + 6..MOD_INFO_OFF + 8].copy_from_slice(&1u16.to_be_bytes());
        data[MOD_INFO_OFF + 16..MOD_INFO_OFF + 20]
            .copy_from_slice(&(NAME_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 20..MOD_INFO_OFF + 24]
            .copy_from_slice(&(NID_TABLE_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 24..MOD_INFO_OFF + 28]
            .copy_from_slice(&(STUB_TABLE_OFF as u32).to_be_bytes());

        data[NAME_OFF..NAME_OFF + 4].copy_from_slice(b"tst\0");
        data[NID_TABLE_OFF..NID_TABLE_OFF + 4].copy_from_slice(&nid.to_be_bytes());
        data[STUB_TABLE_OFF..STUB_TABLE_OFF + 4].copy_from_slice(&0u32.to_be_bytes());

        data
    }

    #[test]
    fn parse_synthetic_elf_round_trips_one_module_one_function() {
        let nid = 0xDEAD_BEEFu32;
        let data = build_synthetic_prx_elf(nid);
        let modules = parse_imports(&data).expect("synthetic ELF must parse");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "tst");
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[0].functions[0].nid, nid);
        assert_eq!(modules[0].functions[0].stub_addr, 260);
    }

    #[test]
    fn parse_rejects_param_header_too_small() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let param_off = 176;
        data[param_off..param_off + 4].copy_from_slice(&16u32.to_be_bytes());
        assert!(matches!(
            parse_imports(&data),
            Err(ImportParseError::ParamHeaderTooSmall(16))
        ));
    }

    #[test]
    fn parse_rejects_unmapped_nid_table_when_function_count_nonzero() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        let unmapped_vaddr: u32 = 0xFFFF_0000;
        data[mod_info_off + 20..mod_info_off + 24].copy_from_slice(&unmapped_vaddr.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(
                err,
                ImportParseError::InvalidNidPtr {
                    vaddr: 0xFFFF_0000,
                    function_count: 1
                }
            ),
            "expected InvalidNidPtr, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_entry_size_below_min() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        data[mod_info_off] = 0;
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::EntryTooSmall { declared: 0 }),
            "expected EntryTooSmall(0), got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_entry_size_below_canonical_min() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        data[mod_info_off] = 0x10;
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::EntryTooSmall { declared: 0x10 }),
            "expected EntryTooSmall(0x10), got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_imports_table_end_below_start() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let param_off = 176;
        let bad_start: u32 = 0x300;
        let bad_end: u32 = 0x200;
        data[param_off + 24..param_off + 28].copy_from_slice(&bad_start.to_be_bytes());
        data[param_off + 28..param_off + 32].copy_from_slice(&bad_end.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(
                err,
                ImportParseError::BadImportsTableRange {
                    start: 0x300,
                    end: 0x200
                }
            ),
            "expected BadImportsTableRange, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_entry_extending_past_imports_table_end() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let param_off = 176;
        // Entry at 208 declares size 0x2C; truncate `imports_table_end`
        // to 208 + 16 so the entry's tail extends past it.
        let truncated_end: u32 = 208 + 16;
        data[param_off + 28..param_off + 32].copy_from_slice(&truncated_end.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert_eq!(
            err,
            ImportParseError::EntryPastImportsTable {
                entry_start: 208,
                entry_size: 0x2C,
                imports_table_end: 224,
            },
            "expected EntryPastImportsTable with the exact truncated end (224), got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_unmapped_name_ptr() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        let unmapped_vaddr: u32 = 0xFFFF_0000;
        data[mod_info_off + 16..mod_info_off + 20].copy_from_slice(&unmapped_vaddr.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::InvalidNamePtr { vaddr: 0xFFFF_0000 }),
            "expected InvalidNamePtr, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_name_missing_nul_within_cap() {
        // Name region is shorter than PRX_NAME_MAX_LEN, so the
        // segment end (320, == TOTAL_SIZE) is the binding cap.
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let name_off = 252;
        for byte in &mut data[name_off..320] {
            *byte = b'A';
        }
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::InvalidNamePtr { .. }),
            "expected InvalidNamePtr from missing NUL, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_unmapped_stub_ptr() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        let unmapped_vaddr: u32 = 0xFFFF_0000;
        data[mod_info_off + 24..mod_info_off + 28].copy_from_slice(&unmapped_vaddr.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(
                err,
                ImportParseError::InvalidStubPtr {
                    vaddr: 0xFFFF_0000,
                    function_count: 1
                }
            ),
            "expected InvalidStubPtr, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_function_count_larger_than_nid_array_in_file() {
        // function_count = 100 but only one u32 of NID array bytes
        // lies within the segment.
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        let inflated: u16 = 100;
        data[mod_info_off + 6..mod_info_off + 8].copy_from_slice(&inflated.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::InvalidNidPtr { .. }),
            "expected InvalidNidPtr from function_count overflow, got {err:?}"
        );
    }

    #[test]
    fn parse_synthetic_elf_function_count_zero_is_variable_only_module() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        let mod_info_off = 208;
        data[mod_info_off + 6..mod_info_off + 8].copy_from_slice(&0u16.to_be_bytes());
        let modules = parse_imports(&data).expect("variable-only module must parse");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "tst");
        assert!(modules[0].functions.is_empty());
    }

    #[test]
    fn no_imports_table_returns_error() {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&0u16.to_be_bytes());

        assert!(
            matches!(parse_imports(&data), Err(ImportParseError::NoImportsTable)),
            "expected NoImportsTable error"
        );
    }

    // -- ppu_prx_library_info path (firmware-PRX layout) --
    //
    // Layout: one PT_LOAD covering the whole file with identity
    // vaddr -> file_offset mapping, no PT_PRX_PARAM. Segment 0's
    // `p_paddr` points at a `ppu_prx_library_info` struct whose
    // `imports_start/end` enclose one PrxImportEntry.
    //
    // File map (all hex):
    //   0x000..0x040  ELF header (phoff=0x40, phentsize=56, phnum=1)
    //   0x040..0x078  Phdr 0    (PT_LOAD; p_paddr = LIB_INFO_OFF)
    //   0x0A0..0x0D4  library_info (52 bytes)
    //   0x0D4..0x100  PrxImportEntry (0x2C bytes)
    //   0x100..0x108  name "tst\0" + pad
    //   0x108..0x10C  NID
    //   0x10C..0x110  stub slot
    fn build_library_info_prx_elf(nid: u32) -> Vec<u8> {
        const TOTAL_SIZE: usize = 320;
        const LIB_INFO_OFF: usize = 0xA0;
        const MOD_INFO_OFF: usize = 0xD4;
        const MOD_INFO_SIZE: u8 = 0x2C;
        const NAME_OFF: usize = 0x100;
        const NID_TABLE_OFF: usize = 0x108;
        const STUB_TABLE_OFF: usize = 0x10C;

        let mut data = vec![0u8; TOTAL_SIZE];
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());

        // Phdr 0: PT_LOAD covering the whole file 1:1, with
        // p_paddr repurposed to point at LIB_INFO_OFF.
        let ph0 = 64usize;
        data[ph0..ph0 + 4].copy_from_slice(&1u32.to_be_bytes());
        // p_offset = 0
        data[ph0 + 8..ph0 + 16].copy_from_slice(&0u64.to_be_bytes());
        // p_vaddr = 0 (identity mapping)
        data[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
        // p_paddr = LIB_INFO_OFF (Sony repurpose)
        data[ph0 + 24..ph0 + 32].copy_from_slice(&(LIB_INFO_OFF as u64).to_be_bytes());
        // p_filesz = TOTAL_SIZE
        data[ph0 + 32..ph0 + 40].copy_from_slice(&(TOTAL_SIZE as u64).to_be_bytes());

        // library_info: imports_start at +44, imports_end at +48
        // (rest of the 52-byte struct stays zero -- attributes /
        // version / name / toc / exports are not consulted by
        // the import-path locator).
        data[LIB_INFO_OFF + 44..LIB_INFO_OFF + 48]
            .copy_from_slice(&(MOD_INFO_OFF as u32).to_be_bytes());
        data[LIB_INFO_OFF + 48..LIB_INFO_OFF + 52]
            .copy_from_slice(&(MOD_INFO_OFF as u32 + MOD_INFO_SIZE as u32).to_be_bytes());

        // PrxImportEntry: size=0x2C, num_func=1, three pointers.
        data[MOD_INFO_OFF] = MOD_INFO_SIZE;
        data[MOD_INFO_OFF + 6..MOD_INFO_OFF + 8].copy_from_slice(&1u16.to_be_bytes());
        data[MOD_INFO_OFF + 16..MOD_INFO_OFF + 20]
            .copy_from_slice(&(NAME_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 20..MOD_INFO_OFF + 24]
            .copy_from_slice(&(NID_TABLE_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 24..MOD_INFO_OFF + 28]
            .copy_from_slice(&(STUB_TABLE_OFF as u32).to_be_bytes());

        data[NAME_OFF..NAME_OFF + 4].copy_from_slice(b"tst\0");
        data[NID_TABLE_OFF..NID_TABLE_OFF + 4].copy_from_slice(&nid.to_be_bytes());
        data[STUB_TABLE_OFF..STUB_TABLE_OFF + 4].copy_from_slice(&0u32.to_be_bytes());

        data
    }

    #[test]
    fn parse_synthetic_library_info_path_round_trips_one_module() {
        let nid = 0xCAFE_BABEu32;
        let data = build_library_info_prx_elf(nid);
        let modules = parse_imports(&data).expect("library_info ELF must parse");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "tst");
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[0].functions[0].nid, nid);
        assert_eq!(modules[0].functions[0].stub_addr, 0x10C);
    }

    #[test]
    fn parse_library_info_p_paddr_past_file_end_is_out_of_bounds() {
        let mut data = build_library_info_prx_elf(0xCAFE_BABE);
        let ph0 = 64usize;
        // p_paddr = TOTAL_SIZE - 10 (310): the 52-byte struct end
        // at +42 lands past the file's last byte.
        data[ph0 + 24..ph0 + 32].copy_from_slice(&310u64.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::OutOfBounds),
            "expected OutOfBounds for library_info past file end, got {err:?}"
        );
    }

    #[test]
    fn parse_library_info_p_paddr_above_u32_max_is_out_of_bounds() {
        let mut data = build_library_info_prx_elf(0xCAFE_BABE);
        let ph0 = 64usize;
        data[ph0 + 24..ph0 + 32].copy_from_slice(&u64::MAX.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::OutOfBounds),
            "expected OutOfBounds for huge p_paddr, got {err:?}"
        );
    }

    #[test]
    fn parse_library_info_bad_imports_range_surfaces_bad_imports_table_range() {
        let mut data = build_library_info_prx_elf(0xCAFE_BABE);
        let lib_info_off = 0xA0;
        let bad_start: u32 = 0x300;
        let bad_end: u32 = 0x200;
        data[lib_info_off + 44..lib_info_off + 48].copy_from_slice(&bad_start.to_be_bytes());
        data[lib_info_off + 48..lib_info_off + 52].copy_from_slice(&bad_end.to_be_bytes());
        let err = parse_imports(&data).unwrap_err();
        assert_eq!(
            err,
            ImportParseError::BadImportsTableRange {
                start: 0x300,
                end: 0x200,
            }
        );
    }

    #[test]
    fn parse_prefers_pt_prx_param_over_library_info_when_both_present() {
        let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
        // Synthetic fixture: TOTAL_SIZE=320, with bytes 0x108..0x140
        // unused. Place library_info there.
        let lib_info_off: usize = 0x108;
        let ph0 = 64usize;
        data[ph0 + 24..ph0 + 32].copy_from_slice(&(lib_info_off as u64).to_be_bytes());
        // library_info with bogus imports range (unmapped high
        // vaddrs). 52-byte struct fits in 0x108..0x13C.
        let bogus_start: u32 = 0xFFFF_0000;
        let bogus_end: u32 = 0xFFFF_0010;
        data[lib_info_off + 44..lib_info_off + 48].copy_from_slice(&bogus_start.to_be_bytes());
        data[lib_info_off + 48..lib_info_off + 52].copy_from_slice(&bogus_end.to_be_bytes());

        let modules = parse_imports(&data).expect("PT_PRX_PARAM precedence must hold");
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "tst");
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[0].functions[0].nid, 0xDEAD_BEEF);
    }

    #[test]
    #[ignore = "requires tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf; \
                run with CELLGOV_RETAIL_FIXTURES=1 cargo test -- --ignored"]
    fn bind_hle_stubs_writes_trampolines() {
        use cellgov_mem::GuestMemory;

        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
            if std::env::var_os("CELLGOV_RETAIL_FIXTURES").is_some() {
                panic!(
                    "CELLGOV_RETAIL_FIXTURES set but {} is absent",
                    path.display()
                );
            }
            return;
        }
        let data = std::fs::read(&path).unwrap();
        let modules = parse_imports(&data).unwrap();

        let required = crate::loader::required_memory_size(&data).unwrap();
        let mem_size = ((required + 0xFFFF) & !0xFFFF) + 0x100000;
        let mut mem = GuestMemory::new(mem_size);
        let mut state = crate::state::PpuState::new();
        crate::loader::load_ppu_elf(&data, &mut mem, &mut state).unwrap();

        let tramp_base = ((required + 0xFFF) & !0xFFF) as u32;
        let bindings = bind_hle_stubs(&modules, &mut mem, tramp_base).expect("retail bind");

        assert_eq!(bindings.len(), 140);

        for (i, binding) in bindings.iter().enumerate() {
            let base = (tramp_base + (i as u32) * TRAMPOLINE_SIZE) as usize;
            let opd_code = u32::from_be_bytes([
                mem.as_bytes()[base],
                mem.as_bytes()[base + 1],
                mem.as_bytes()[base + 2],
                mem.as_bytes()[base + 3],
            ]);
            assert_eq!(
                opd_code,
                (base + 8) as u32,
                "trampoline {i} OPD code_addr mismatch"
            );

            let code_base = base + 8;
            let word = |off: usize| -> u32 {
                let p = code_base + off;
                u32::from_be_bytes([
                    mem.as_bytes()[p],
                    mem.as_bytes()[p + 1],
                    mem.as_bytes()[p + 2],
                    mem.as_bytes()[p + 3],
                ])
            };
            let expected_nr = HLE_SYSCALL_BASE + binding.index;
            let hi = (expected_nr >> 16) & 0xFFFF;
            let lo = expected_nr & 0xFFFF;
            let expected_lis = (15 << 26) | (11 << 21) | hi;
            let expected_ori = (24 << 26) | (11 << 21) | (11 << 16) | lo;
            assert_eq!(
                word(0),
                expected_lis,
                "trampoline {i} lis r11 mismatch: got 0x{:08x}",
                word(0)
            );
            assert_eq!(
                word(4),
                expected_ori,
                "trampoline {i} ori r11 mismatch: got 0x{:08x}",
                word(4)
            );
            assert_eq!(word(8), 0x4400_0002, "trampoline {i} sc mismatch");
            assert_eq!(word(12), 0x4E80_0020, "trampoline {i} blr mismatch");
        }

        let last = bindings.last().unwrap();
        let got_addr = last.stub_addr as usize;
        let patched = u32::from_be_bytes([
            mem.as_bytes()[got_addr],
            mem.as_bytes()[got_addr + 1],
            mem.as_bytes()[got_addr + 2],
            mem.as_bytes()[got_addr + 3],
        ]);
        let expected_tramp = tramp_base + last.index * TRAMPOLINE_SIZE;
        assert_eq!(patched, expected_tramp, "last GOT entry not patched");
    }

    fn make_modules(specs: &[(&str, &[(u32, u32)])]) -> Vec<ImportedModule> {
        specs
            .iter()
            .map(|(name, funcs)| ImportedModule {
                name: (*name).to_string(),
                functions: funcs
                    .iter()
                    .map(|&(nid, stub)| ImportedFunction {
                        nid,
                        stub_addr: stub,
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn bind_synthetic_legacy24_writes_full_trampolines_and_patches_got() {
        use cellgov_mem::{GuestMemory, PageSize, Region};
        let mut mem =
            GuestMemory::from_regions(vec![Region::new(0, 0x0100_0000, "main", PageSize::Page64K)])
                .unwrap();
        let modules = make_modules(&[
            (
                "modA",
                &[(0x1111_1111, 0x0080_0000), (0x2222_2222, 0x0080_0004)],
            ),
            ("modB", &[(0x3333_3333, 0x0080_0008)]),
        ]);
        let tramp_base: u32 = 0x0090_0000;
        let bindings = bind_hle_stubs(&modules, &mut mem, tramp_base).expect("bind");
        assert_eq!(bindings.len(), 3);

        for binding in &bindings {
            let opd_addr = tramp_base + binding.index * TRAMPOLINE_SIZE;
            let body_addr = opd_addr + 8;
            // GOT slot value == opd_addr.
            let stub_range = ByteRange::new(GuestAddr::new(binding.stub_addr as u64), 4).unwrap();
            let got_bytes = mem.read(stub_range).unwrap();
            assert_eq!(u32::from_be_bytes(got_bytes.try_into().unwrap()), opd_addr);
            // OPD code field == body_addr.
            let opd_range = ByteRange::new(GuestAddr::new(opd_addr as u64), 4).unwrap();
            let opd_bytes = mem.read(opd_range).unwrap();
            assert_eq!(u32::from_be_bytes(opd_bytes.try_into().unwrap()), body_addr);
        }
    }

    #[test]
    fn bind_atomic_batch_rejects_whole_batch_on_unmapped_got_slot() {
        // First binding's GOT slot is in writable memory; second's
        // is inside a ReservedStrict region. The atomic-batch
        // contract requires that no byte be committed.
        use cellgov_mem::{GuestMemory, PageSize, Region, RegionAccess};
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x0100_0000, "main", PageSize::Page64K),
            Region::with_access(
                0x4000_0000,
                0x0001_0000,
                "ro",
                PageSize::Page64K,
                RegionAccess::ReservedStrict,
            ),
        ])
        .unwrap();
        let modules = make_modules(&[
            ("modOK", &[(0xAAAA_0001, 0x0080_0000)]),
            ("modBAD", &[(0xBBBB_0001, 0x4000_0010)]),
        ]);
        let tramp_base: u32 = 0x0090_0000;
        let before = mem.content_hash();
        let err = bind_hle_stubs(&modules, &mut mem, tramp_base).unwrap_err();
        match err {
            BindError::StagingCommitFailed { staged_count, .. } => {
                // 2 bindings * 3 writes per binding (OPD, body, GOT).
                assert_eq!(
                    staged_count, 6,
                    "atomic batch must carry every binding's writes"
                );
            }
            other => panic!("expected StagingCommitFailed, got {other:?}"),
        }
        assert_eq!(
            mem.content_hash(),
            before,
            "atomic-batch contract: a rejected drain must leave guest memory unchanged"
        );
    }

    #[test]
    fn bind_layout_overflow_rejects_with_typed_error() {
        // legacy_base + TRAMPOLINE_SIZE = u32::MAX + 8: binding 0's
        // footprint trips before binding 1's start arithmetic.
        use cellgov_mem::{GuestMemory, PageSize, Region};
        let mut mem =
            GuestMemory::from_regions(vec![Region::new(0, 0x0100_0000, "main", PageSize::Page64K)])
                .unwrap();
        let modules = make_modules(&[
            ("modA", &[(0x1111_1111, 0x0080_0000)]),
            ("modB", &[(0x2222_2222, 0x0080_0004)]),
        ]);
        let err =
            bind_hle_stubs(&modules, &mut mem, u32::MAX - 16).expect_err("overflow must trip");
        assert!(
            matches!(
                err,
                BindError::LayoutOverflow {
                    binding_index: 0,
                    kind: "legacy_footprint",
                }
            ),
            "expected LayoutOverflow at binding_index=0 kind=legacy_footprint, got {err:?}"
        );
    }

    #[test]
    fn bind_single_binding_footprint_end_overflow_trips_layout_error() {
        // legacy_base = u32::MAX - 16: tramp_0 (u32::MAX - 16) and
        // body_0 (u32::MAX - 8) both fit; only tramp + TRAMPOLINE_SIZE
        // (= u32::MAX + 8) overflows, isolating the footprint check.
        use cellgov_mem::{GuestMemory, PageSize, Region};
        let mut mem =
            GuestMemory::from_regions(vec![Region::new(0, 0x0100_0000, "main", PageSize::Page64K)])
                .unwrap();
        let modules = make_modules(&[("modA", &[(0x1111_1111, 0x0080_0000)])]);
        let err =
            bind_hle_stubs(&modules, &mut mem, u32::MAX - 16).expect_err("footprint overflow");
        assert!(
            matches!(
                err,
                BindError::LayoutOverflow {
                    binding_index: 0,
                    kind: "legacy_footprint",
                }
            ),
            "expected LayoutOverflow kind=legacy_footprint, got {err:?}"
        );
    }

    #[test]
    fn bind_ps3_spec_opd_footprint_end_overflow_trips_layout_error() {
        // opd_base = u32::MAX - 4: opd_0 fits, opd + 8 overflows.
        use cellgov_mem::{GuestMemory, PageSize, Region};
        let mut mem =
            GuestMemory::from_regions(vec![Region::new(0, 0x0100_0000, "main", PageSize::Page64K)])
                .unwrap();
        let modules = make_modules(&[("modA", &[(0x1111_1111, 0x0080_0000)])]);
        let err = bind_hle_stubs_with_layout(
            &modules,
            &mut mem,
            HleLayout::Ps3Spec {
                opd_base: u32::MAX - 4,
                body_base: 0x0090_0000,
            },
            0,
        )
        .expect_err("opd footprint overflow");
        assert!(
            matches!(
                err,
                BindError::LayoutOverflow {
                    binding_index: 0,
                    kind: "opd_footprint",
                }
            ),
            "expected LayoutOverflow kind=opd_footprint, got {err:?}"
        );
    }

    /// Build a synthetic PRX with a larger PT_LOAD than
    /// [`build_synthetic_prx_elf`] so the name-pointer region has
    /// more than [`PRX_NAME_MAX_LEN`] bytes of segment remainder.
    /// `name_byte` controls what fills the name region; `b'A'` plus
    /// `PRX_NAME_MAX_LEN + 1` bytes exercises the 256-byte cap.
    fn build_large_name_prx_elf(name_byte: u8) -> Vec<u8> {
        const TOTAL_SIZE: usize = 1024;
        const PARAM_OFF: usize = 176;
        const MOD_INFO_OFF: usize = 208;
        const MOD_INFO_SIZE: u8 = 0x2C;
        const NAME_OFF: usize = 256;
        const NID_TABLE_OFF: usize = 600;
        const STUB_TABLE_OFF: usize = 604;

        let mut data = vec![0u8; TOTAL_SIZE];
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&2u16.to_be_bytes());

        let ph0 = 64usize;
        data[ph0..ph0 + 4].copy_from_slice(&1u32.to_be_bytes());
        data[ph0 + 8..ph0 + 16].copy_from_slice(&0u64.to_be_bytes());
        data[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
        data[ph0 + 32..ph0 + 40].copy_from_slice(&(TOTAL_SIZE as u64).to_be_bytes());

        let ph1 = 64 + 56;
        data[ph1..ph1 + 4].copy_from_slice(&PT_PRX_PARAM.to_be_bytes());
        data[ph1 + 8..ph1 + 16].copy_from_slice(&(PARAM_OFF as u64).to_be_bytes());

        data[PARAM_OFF..PARAM_OFF + 4].copy_from_slice(&0x40u32.to_be_bytes());
        data[PARAM_OFF + 4..PARAM_OFF + 8].copy_from_slice(&PRX_PARAM_MAGIC.to_be_bytes());
        data[PARAM_OFF + 24..PARAM_OFF + 28].copy_from_slice(&(MOD_INFO_OFF as u32).to_be_bytes());
        data[PARAM_OFF + 28..PARAM_OFF + 32]
            .copy_from_slice(&(MOD_INFO_OFF as u32 + MOD_INFO_SIZE as u32).to_be_bytes());

        data[MOD_INFO_OFF] = MOD_INFO_SIZE;
        data[MOD_INFO_OFF + 6..MOD_INFO_OFF + 8].copy_from_slice(&1u16.to_be_bytes());
        data[MOD_INFO_OFF + 16..MOD_INFO_OFF + 20]
            .copy_from_slice(&(NAME_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 20..MOD_INFO_OFF + 24]
            .copy_from_slice(&(NID_TABLE_OFF as u32).to_be_bytes());
        data[MOD_INFO_OFF + 24..MOD_INFO_OFF + 28]
            .copy_from_slice(&(STUB_TABLE_OFF as u32).to_be_bytes());

        // Fill the name region with `name_byte` for 257 bytes -- one
        // past the 256-byte cap -- with no NUL terminator inside.
        // The PT_LOAD's remainder from NAME_OFF (= 1024 - 256 = 768)
        // is much larger than the cap, so the cap is the binding
        // constraint.
        for byte in &mut data[NAME_OFF..NAME_OFF + 257] {
            *byte = name_byte;
        }
        data[NID_TABLE_OFF..NID_TABLE_OFF + 4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        data[STUB_TABLE_OFF..STUB_TABLE_OFF + 4].copy_from_slice(&0u32.to_be_bytes());

        data
    }

    #[test]
    fn parse_rejects_name_unterminated_within_prx_name_max_len_cap() {
        // Segment remainder (768) > PRX_NAME_MAX_LEN (256), so the
        // cap is the binding clause; 257 non-NUL bytes hit it.
        let data = build_large_name_prx_elf(b'A');
        let err = parse_imports(&data).unwrap_err();
        assert!(
            matches!(err, ImportParseError::InvalidNamePtr { vaddr: 256 }),
            "expected InvalidNamePtr from PRX_NAME_MAX_LEN cap, got {err:?}"
        );
    }

    #[test]
    fn bind_atomic_batch_rejects_when_opd_lands_in_reserved_strict() {
        // OPD/body land in ReservedStrict, GOT slot in writable
        // memory: the drain rejects the OPD/body writes and the
        // atomic-batch contract requires zero bytes be committed.
        use cellgov_mem::{GuestMemory, PageSize, Region, RegionAccess};
        let mut mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x0100_0000, "main", PageSize::Page64K),
            Region::with_access(
                0x4000_0000,
                0x0001_0000,
                "ro",
                PageSize::Page64K,
                RegionAccess::ReservedStrict,
            ),
        ])
        .unwrap();
        let modules = make_modules(&[("modA", &[(0xAAAA_0001, 0x0080_0000)])]);
        let before = mem.content_hash();
        let err =
            bind_hle_stubs(&modules, &mut mem, 0x4000_0010).expect_err("ro trampoline must trip");
        match err {
            BindError::StagingCommitFailed { staged_count, .. } => {
                assert_eq!(staged_count, 3);
            }
            other => panic!("expected StagingCommitFailed, got {other:?}"),
        }
        assert_eq!(
            mem.content_hash(),
            before,
            "atomic-batch contract: OPD/body in ReservedStrict must leave guest memory unchanged"
        );
    }

    #[test]
    fn bind_empty_modules_returns_empty_bindings_with_no_writes() {
        use cellgov_mem::{GuestMemory, PageSize, Region};
        let mut mem =
            GuestMemory::from_regions(vec![Region::new(0, 0x0100_0000, "main", PageSize::Page64K)])
                .unwrap();
        let before = mem.content_hash();
        let bindings = bind_hle_stubs(&[], &mut mem, 0x0090_0000).expect("empty input");
        assert!(bindings.is_empty());
        assert_eq!(mem.content_hash(), before, "empty bind must not write");
    }
}
