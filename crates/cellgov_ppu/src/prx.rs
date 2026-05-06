//! PS3 PRX import-table parser and HLE trampoline binder.
//!
//! Walks `PrxParamHeader` in PT_0x60000002 to enumerate imported
//! modules / NIDs / GOT slots, then writes HLE trampolines into guest
//! memory and patches each GOT slot to point at its OPD.

use crate::loader;

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
    /// No `PT_PRX_PARAM` program header was found.
    NoPrxParam,
    /// `PrxParamHeader` magic did not match `PRX_PARAM_MAGIC`.
    BadMagic(u32),
    /// `header_size` declared smaller than the
    /// imports_table_start/end fields at +24/+28.
    ParamHeaderTooSmall(u32),
    /// A read or virtual-address resolution went past the file or segment.
    OutOfBounds,
}

const PT_PRX_PARAM: u32 = 0x6000_0002;
const PRX_PARAM_MAGIC: u32 = 0x1b43_4cec;

/// Enumerate every imported module and its (NID, GOT slot) entries.
pub fn parse_imports(data: &[u8]) -> Result<Vec<ImportedModule>, ImportParseError> {
    if data.len() < 64 {
        return Err(ImportParseError::OutOfBounds);
    }
    let phoff = loader::read_u64(data, 32) as usize;
    let phentsize = loader::read_u16(data, 54) as usize;
    let phnum = loader::read_u16(data, 56) as usize;

    let mut prx_param_offset = None;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            return Err(ImportParseError::OutOfBounds);
        }
        let p_type = loader::read_u32(data, base);
        if p_type == PT_PRX_PARAM {
            let p_offset = loader::read_u64(data, base + 8) as usize;
            prx_param_offset = Some(p_offset);
            break;
        }
    }

    let param_off = prx_param_offset.ok_or(ImportParseError::NoPrxParam)?;
    if param_off + 32 > data.len() {
        return Err(ImportParseError::OutOfBounds);
    }

    // PrxParamHeader: { u32 header_size, magic, version, reserved0,
    //   exports_table_start, exports_table_end,
    //   imports_table_start, imports_table_end }.
    let magic = loader::read_u32(data, param_off + 4);
    if magic != PRX_PARAM_MAGIC {
        return Err(ImportParseError::BadMagic(magic));
    }
    // Reject a header that ends before the imports-table fields at
    // +24/+28 so we don't read those offsets against unrelated bytes.
    let declared_size = loader::read_u32(data, param_off);
    if declared_size < 32 {
        return Err(ImportParseError::ParamHeaderTooSmall(declared_size));
    }

    let imports_table_start = loader::read_u32(data, param_off + 24) as usize;
    let imports_table_end = loader::read_u32(data, param_off + 28) as usize;

    let segments = build_segment_map(data, phoff, phentsize, phnum);

    let mut modules = Vec::new();
    let mut addr = imports_table_start;
    while addr < imports_table_end {
        let foff = vaddr_to_file(&segments, addr).ok_or(ImportParseError::OutOfBounds)?;
        if foff >= data.len() {
            return Err(ImportParseError::OutOfBounds);
        }

        // PrxImportEntry: { u8 entry_size, reserved0, u16 module_version,
        //   attributes, function_count, variable_count, tls_var_count,
        //   u8 name_hash_byte, tls_hash_byte, u8[2] reserved1,
        //   u32 name_ptr, nid_ptr, stub_ptr }.
        let entry_size = data[foff] as usize;
        if entry_size == 0 {
            break;
        }
        if foff + entry_size > data.len() {
            return Err(ImportParseError::OutOfBounds);
        }

        let function_count = loader::read_u16(data, foff + 6) as usize;
        let name_ptr = loader::read_u32(data, foff + 16) as usize;
        let nid_ptr = loader::read_u32(data, foff + 20) as usize;
        let stub_ptr = loader::read_u32(data, foff + 24) as usize;

        let name = read_cstring(data, &segments, name_ptr);

        let mut functions = Vec::with_capacity(function_count);
        // Variable-only modules (function_count == 0) may carry an
        // unmapped nid_ptr; resolving it would silently fall back to
        // file offset 0 (the ELF header) and yield garbage NIDs.
        if function_count > 0 {
            let nid_foff =
                vaddr_to_file(&segments, nid_ptr).ok_or(ImportParseError::OutOfBounds)?;
            for i in 0..function_count {
                let nid_off = nid_foff + i * 4;
                if nid_off + 4 > data.len() {
                    break;
                }
                let nid = loader::read_u32(data, nid_off);
                let stub_addr = (stub_ptr + i * 4) as u32;
                functions.push(ImportedFunction { nid, stub_addr });
            }
        }

        modules.push(ImportedModule { name, functions });
        addr += entry_size;
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
/// Consumed by the PRX binder and by `dump-imports` to tag each
/// import `impl` vs `stub`. Entries resolve through
/// `cellgov_ps3_abi::nid::*` (hex lives only in the leaf, verified
/// against `nid_sha1(name)` at compile time). Grouped by module;
/// callers use `contains`, not binary search.
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

/// [`bind_hle_stubs_with_layout`] with [`HleLayout::Legacy24`].
pub fn bind_hle_stubs(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    trampoline_base: u32,
) -> Vec<HleBinding> {
    bind_hle_stubs_with_layout(modules, memory, HleLayout::Legacy24, trampoline_base)
}

/// Write HLE OPDs and body trampolines into guest memory per `layout`
/// and patch each imported GOT slot to point at its OPD.
///
/// Every imported function is bound regardless of `HLE_IMPLEMENTED_NIDS`
/// membership; the runtime dispatcher handles unimplemented syscalls.
///
/// # Panics
/// Panics if any OPD, body, or GOT `apply_commit` fails -- placement
/// landed outside a writable region, and returning a partial binding
/// vec would silently corrupt the dispatch surface.
pub fn bind_hle_stubs_with_layout(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    layout: HleLayout,
    legacy_base: u32,
) -> Vec<HleBinding> {
    use cellgov_ps3_abi::syscall_namespace::SyscallNamespace;
    use cellgov_ps3_abi::trampoline_codegen::{
        encode_blr, encode_lis_ori_sc, encode_ps3_packed_opd,
    };

    let mut bindings = Vec::new();
    let mut legacy_offset = 0u32;

    for module in modules {
        for func in &module.functions {
            let hle_index = bindings.len() as u32;
            // try_encode produces a descriptive panic if a title
            // exhausts the HleImport namespace (~0x80000 imports);
            // encode would emit a generic debug-only assert.
            let syscall_nr_u64 = SyscallNamespace::HleImport
                .try_encode(hle_index)
                .unwrap_or_else(|| {
                    panic!(
                        "HLE binding count exceeded the HleImport namespace capacity \
                         (hle_index={hle_index}, namespace upper bound=0x{:x}); \
                         widen SyscallNamespace::HleImport before retrying",
                        SyscallNamespace::HleImport.range().1,
                    )
                });
            let syscall_nr =
                u32::try_from(syscall_nr_u64).expect("HleImport namespace fits in u32");
            let (opd_addr, body_addr) = match layout {
                HleLayout::Legacy24 => {
                    let tramp = legacy_base + legacy_offset;
                    legacy_offset += TRAMPOLINE_SIZE;
                    (tramp, tramp + 8)
                }
                HleLayout::Ps3Spec {
                    opd_base,
                    body_base,
                } => (opd_base + hle_index * 8, body_base + hle_index * 16),
            };

            // OPD: { body_addr, toc=0 } in RPCS3-packed 8-byte shape,
            // not 24-byte ELFv1. The binder dereferences via packed
            // convention.
            let opd_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(opd_addr as u64), 8)
                    .expect("OPD range fits in u64");
            let opd_bytes = encode_ps3_packed_opd(body_addr, 0);
            memory
                .apply_commit(opd_range, &opd_bytes)
                .expect("HLE OPD write failed; trampoline_base must point at a writable region");

            // Body: lis r11, hi; ori r11, r11, lo; sc 0; blr.
            let body_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(body_addr as u64), 16)
                    .expect("body range fits in u64");
            let lis_ori_sc = encode_lis_ori_sc(syscall_nr);
            let blr = encode_blr();
            let mut body_bytes = [0u8; 16];
            body_bytes[0..12].copy_from_slice(&lis_ori_sc);
            body_bytes[12..16].copy_from_slice(&blr);
            memory
                .apply_commit(body_range, &body_bytes)
                .expect("HLE body write failed; trampoline_base must point at a writable region");

            let got_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(func.stub_addr as u64), 4)
                    .expect("GOT slot range fits in u64");
            memory
                .apply_commit(got_range, &opd_addr.to_be_bytes())
                .expect("GOT patch failed; the import's stub_addr must lie in a writable region");

            bindings.push(HleBinding {
                index: hle_index,
                module: module.name.clone(),
                nid: func.nid,
                stub_addr: func.stub_addr,
            });
        }
    }

    bindings
}

// -- Internal helpers --

struct Segment {
    vaddr: usize,
    file_offset: usize,
    size: usize,
}

fn build_segment_map(data: &[u8], phoff: usize, phentsize: usize, phnum: usize) -> Vec<Segment> {
    let mut segs = Vec::new();
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            break;
        }
        let p_type = loader::read_u32(data, base);
        if p_type != 1 {
            continue;
        }
        let p_offset = loader::read_u64(data, base + 8) as usize;
        let p_vaddr = loader::read_u64(data, base + 16) as usize;
        let p_filesz = loader::read_u64(data, base + 32) as usize;
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
        if vaddr >= seg.vaddr && vaddr < seg.vaddr + seg.size {
            return Some(vaddr - seg.vaddr + seg.file_offset);
        }
    }
    None
}

fn read_cstring(data: &[u8], segments: &[Segment], vaddr: usize) -> String {
    let foff = match vaddr_to_file(segments, vaddr) {
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
    fn parse_retail_eboot_imports() {
        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
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
        assert!(matches!(
            parse_imports(&data),
            Err(ImportParseError::OutOfBounds)
        ));
    }

    #[test]
    fn no_prx_param_returns_error() {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&64u64.to_be_bytes());
        data[54..56].copy_from_slice(&56u16.to_be_bytes());
        data[56..58].copy_from_slice(&0u16.to_be_bytes());

        assert!(
            matches!(parse_imports(&data), Err(ImportParseError::NoPrxParam)),
            "expected NoPrxParam error"
        );
    }

    #[test]
    fn bind_hle_stubs_writes_trampolines() {
        use cellgov_mem::GuestMemory;

        let path =
            std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
        if !path.exists() {
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
        let bindings = bind_hle_stubs(&modules, &mut mem, tramp_base);

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
}
