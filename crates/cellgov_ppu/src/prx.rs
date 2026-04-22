//! PS3 PRX import/export table parser.
//!
//! Parses the `ppu_proc_prx_param_t` structure from the PT_0x60000002
//! program header and walks the `ppu_prx_module_info` entries in the
//! import (libstub) array. This is the standard PS3 SDK format used
//! by every PS3 executable to declare its imported system library
//! functions.
//!
//! The parser works on raw ELF bytes and committed guest memory. It
//! does not depend on any specific game title.

use crate::loader;

/// A single imported module with its functions.
#[derive(Debug, Clone)]
pub struct ImportedModule {
    /// Module name (e.g., "cellSysutil", "sysPrxForUser").
    pub name: String,
    /// Imported functions: (NID, OPD guest address).
    pub functions: Vec<ImportedFunction>,
}

/// A single imported function.
#[derive(Debug, Clone, Copy)]
pub struct ImportedFunction {
    /// Function NID (Numeric ID) -- a 32-bit hash identifying the function.
    pub nid: u32,
    /// Guest address of the GOT-style slot for this import. The
    /// binder patches this slot with the guest address of an OPD
    /// (function descriptor) so callers dereference it as a normal
    /// PPC function pointer.
    pub stub_addr: u32,
}

/// Why import parsing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportParseError {
    /// No PT_0x60000002 program header found.
    NoPrxParam,
    /// The ppu_proc_prx_param_t magic does not match 0x1b434cec.
    BadMagic(u32),
    /// A read went out of bounds.
    OutOfBounds,
}

/// PT_0x60000002 program header type.
const PT_PRX_PARAM: u32 = 0x6000_0002;

/// Expected magic value in ppu_proc_prx_param_t.
const PRX_PARAM_MAGIC: u32 = 0x1b43_4cec;

/// Parse all imported modules from a PS3 ELF binary.
///
/// Reads the PT_0x60000002 program header to locate the
/// `ppu_proc_prx_param_t`, then walks the libstub array to enumerate
/// every imported module and its function NIDs + OPD addresses.
///
/// `data` is the full ELF image; addresses inside the PRX param
/// structure are resolved against the ELF's own segments.
pub fn parse_imports(data: &[u8]) -> Result<Vec<ImportedModule>, ImportParseError> {
    // Find PT_0x60000002 program header.
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

    // ppu_proc_prx_param_t layout:
    //   u32 size
    //   u32 magic (0x1b434cec)
    //   u32 version
    //   u32 unk0
    //   u32 libent_start (exports)
    //   u32 libent_end
    //   u32 libstub_start (imports)
    //   u32 libstub_end
    let magic = loader::read_u32(data, param_off + 4);
    if magic != PRX_PARAM_MAGIC {
        return Err(ImportParseError::BadMagic(magic));
    }

    let libstub_start = loader::read_u32(data, param_off + 24) as usize;
    let libstub_end = loader::read_u32(data, param_off + 28) as usize;

    // Build a segment map for vaddr-to-file-offset translation.
    let segments = build_segment_map(data, phoff, phentsize, phnum);

    // Walk the libstub array. Each entry is a ppu_prx_module_info.
    let mut modules = Vec::new();
    let mut addr = libstub_start;
    while addr < libstub_end {
        let foff = vaddr_to_file(&segments, addr).ok_or(ImportParseError::OutOfBounds)?;
        if foff >= data.len() {
            return Err(ImportParseError::OutOfBounds);
        }

        // ppu_prx_module_info layout:
        //   u8  size
        //   u8  unk0
        //   u16 version
        //   u16 attributes
        //   u16 num_func
        //   u16 num_var
        //   u16 num_tlsvar
        //   u8  info_hash
        //   u8  info_tlshash
        //   u8  unk1[2]
        //   u32 name_ptr
        //   u32 nid_ptr
        //   u32 stub_ptr
        let entry_size = data[foff] as usize;
        if entry_size == 0 {
            break;
        }
        if foff + entry_size > data.len() {
            return Err(ImportParseError::OutOfBounds);
        }

        let num_func = loader::read_u16(data, foff + 6) as usize;
        let name_ptr = loader::read_u32(data, foff + 16) as usize;
        let nid_ptr = loader::read_u32(data, foff + 20) as usize;
        let stub_ptr = loader::read_u32(data, foff + 24) as usize;

        // Read module name.
        let name = read_cstring(data, &segments, name_ptr);

        // Read NIDs and stub OPD addresses.
        let mut functions = Vec::with_capacity(num_func);
        let nid_foff = vaddr_to_file(&segments, nid_ptr).unwrap_or(0);
        for i in 0..num_func {
            let nid_off = nid_foff + i * 4;
            if nid_off + 4 > data.len() {
                break;
            }
            let nid = loader::read_u32(data, nid_off);
            let stub_addr = (stub_ptr + i * 4) as u32;
            functions.push(ImportedFunction { nid, stub_addr });
        }

        modules.push(ImportedModule { name, functions });
        addr += entry_size;
    }

    Ok(modules)
}

/// Summary of parsed imports for diagnostics.
pub fn import_summary(modules: &[ImportedModule]) -> String {
    let total_funcs: usize = modules.iter().map(|m| m.functions.len()).sum();
    let mut out = format!("{} modules, {} functions:\n", modules.len(), total_funcs);
    for m in modules {
        out.push_str(&format!("  {} ({} functions)\n", m.name, m.functions.len()));
    }
    out
}

/// Base syscall number for HLE import stubs. Real LV2 syscalls use
/// numbers below 1024; HLE stubs start at 0x10000 to avoid collision.
pub const HLE_SYSCALL_BASE: u32 = 0x10000;

/// NIDs for which CellGov ships a dedicated HLE implementation.
///
/// Consumed by two call sites that must stay in concordant:
///
/// 1. `cellgov_cli`'s PRX binder keeps an HLE trampoline for any
///    NID in this list even when the loaded firmware PRX exports
///    a real body for it, because the real body depends on
///    module_start completing full initialization (TLS, heap
///    arenas) that may not have happened yet.
/// 2. `cellgov_cli`'s `dump-imports` inventory tool marks each
///    import as `impl` or `stub` based on whether its NID appears
///    here.
///
/// Previously duplicated across both call sites. Promoted to a
/// single library-level source of truth so adding a new HLE
/// implementation in `cellgov_core::hle::dispatch_hle` only
/// requires updating this list once.
///
/// Ordering is by NID value for stable diffing; no runtime code
/// depends on the order.
pub const HLE_IMPLEMENTED_NIDS: &[u32] = &[
    0x055bd74d, // cellGcmGetTiledPitchSize
    0x15bae46b, // _cellGcmInitBody
    0xa547adde, // cellGcmGetControlRegister
    0xe315a0b2, // cellGcmGetConfiguration
    0xf80196c1, // cellGcmGetLabelAddress
    0x744680a2, // sys_initialize_tls
    0xbdb18f83, // _sys_malloc
    0xf7f7fb20, // _sys_free
    0x68b9b011, // _sys_memset
    0xe6f2c1e7, // sys_process_exit
    0xb2fcf2c8, // _sys_heap_create_heap
    0x2f85c0ef, // sys_lwmutex_create
    0x1573dc3f, // sys_lwmutex_lock
    0xc3476d0c, // sys_lwmutex_destroy
    0x1bc200f4, // sys_lwmutex_unlock
    0xaeb78725, // sys_lwmutex_trylock
    0x8461e528, // sys_time_get_system_time
    0x350d454e, // sys_ppu_thread_get_id
    0x24a1ea07, // sys_ppu_thread_create
    0x4f7172c9, // sys_process_is_stack
    0xa2c7ba64, // sys_prx_exitspawn_with_level
];

/// Bind result: maps each HLE index to its module and NID.
#[derive(Debug, Clone)]
pub struct HleBinding {
    /// HLE index (0-based, added to HLE_SYSCALL_BASE for the syscall number).
    pub index: u32,
    /// Module name.
    pub module: String,
    /// Function NID.
    pub nid: u32,
    /// Guest address of the GOT entry that was patched.
    pub stub_addr: u32,
}

/// Size of each HLE trampoline in guest memory (bytes) for the
/// `Legacy24` layout. The `Ps3Spec` layout uses a different split
/// (8-byte OPD plus a separate 16-byte body per binding) and does
/// not use this constant.
pub const TRAMPOLINE_SIZE: u32 = 24;

/// Layout strategy for HLE trampolines.
#[derive(Debug, Clone, Copy)]
pub enum HleLayout {
    /// Legacy: 24-byte trampolines packed at `trampoline_base`. Each
    /// trampoline carries its own inline `lis/ori/sc/blr` body. The
    /// OPD points to the inline body. Simple and self-contained, but
    /// the resulting GOT pointer values do not match RPCS3, which
    /// uses a different layout.
    Legacy24,
    /// PS3-spec: 8-byte OPDs packed at `opd_base`, body trampolines
    /// packed at `body_base`. Each 8-byte OPD = `{ body_addr, toc=0 }`
    /// where `body_addr` points to a separate per-NID 16-byte body
    /// (lis/ori/sc/blr). This matches RPCS3's HLE OPD shape (8-byte
    /// stride in user memory) and the byte size of each GOT entry
    /// becomes a packed pointer rather than a 24-byte-aligned one.
    Ps3Spec {
        /// Guest address of the first 8-byte OPD slot. Subsequent
        /// bindings get slots at `opd_base + index * 8`.
        opd_base: u32,
        /// Guest address of the first 16-byte body trampoline.
        /// Subsequent bindings get bodies at `body_base + index * 16`.
        body_base: u32,
    },
}

/// Write HLE stub trampolines into guest memory and patch each
/// imported function's GOT entry to point at the trampoline.
///
/// Thin wrapper around [`bind_hle_stubs_with_layout`] that selects
/// the [`HleLayout::Legacy24`] packing (24 bytes per binding at
/// `trampoline_base`).
pub fn bind_hle_stubs(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    trampoline_base: u32,
) -> Vec<HleBinding> {
    bind_hle_stubs_with_layout(modules, memory, HleLayout::Legacy24, trampoline_base)
}

/// Write HLE stub trampolines using the configured layout.
///
/// In `Ps3Spec` mode, 8-byte OPDs land at `opd_base` (in stride-8
/// order matching RPCS3's `vm::alloc(N*8, vm::main)` HLE table) and
/// body trampolines land at `body_base` with stride 16. The GOT
/// entries point at the OPDs.
pub fn bind_hle_stubs_with_layout(
    modules: &[ImportedModule],
    memory: &mut cellgov_mem::GuestMemory,
    layout: HleLayout,
    legacy_base: u32,
) -> Vec<HleBinding> {
    let mut bindings = Vec::new();
    let mut offset = 0u32;

    for module in modules {
        for func in &module.functions {
            let hle_index = bindings.len() as u32;
            let syscall_nr = HLE_SYSCALL_BASE + hle_index;
            let (opd_addr, body_addr) = match layout {
                HleLayout::Legacy24 => {
                    let tramp = legacy_base + offset;
                    (tramp, tramp + 8)
                }
                HleLayout::Ps3Spec {
                    opd_base,
                    body_base,
                } => (opd_base + hle_index * 8, body_base + hle_index * 16),
            };

            let hi = (syscall_nr >> 16) & 0xFFFF;
            let lo = syscall_nr & 0xFFFF;
            // lis r11, hi  =  addis r11, r0, hi
            let lis_r11: u32 = (15 << 26) | (11 << 21) | hi;
            // ori r11, r11, lo
            let ori_r11: u32 = (24 << 26) | (11 << 21) | (11 << 16) | lo;
            let sc: u32 = 0x4400_0002;
            let blr: u32 = 0x4E80_0020;

            // Write OPD: { body_addr, toc=0 }.
            let opd_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(opd_addr as u64), 8);
            if let Some(range) = opd_range {
                let mut bytes = [0u8; 8];
                bytes[0..4].copy_from_slice(&body_addr.to_be_bytes());
                bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
                let _ = memory.apply_commit(range, &bytes);
            }
            // Write body: lis r11, hi; ori r11, r11, lo; sc; blr.
            let body_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(body_addr as u64), 16);
            if let Some(range) = body_range {
                let mut bytes = [0u8; 16];
                bytes[0..4].copy_from_slice(&lis_r11.to_be_bytes());
                bytes[4..8].copy_from_slice(&ori_r11.to_be_bytes());
                bytes[8..12].copy_from_slice(&sc.to_be_bytes());
                bytes[12..16].copy_from_slice(&blr.to_be_bytes());
                let _ = memory.apply_commit(range, &bytes);
            }

            // Patch the GOT entry to point at the OPD.
            let got_range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(func.stub_addr as u64), 4);
            if let Some(range) = got_range {
                let _ = memory.apply_commit(range, &opd_addr.to_be_bytes());
            }

            bindings.push(HleBinding {
                index: hle_index,
                module: module.name.clone(),
                nid: func.nid,
                stub_addr: func.stub_addr,
            });

            offset += TRAMPOLINE_SIZE;
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
            // PT_LOAD only
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
        // sys_initialize_tls is called by every PS3 ELF boot; if
        // it drops off this list every boot silently downgrades.
        assert!(HLE_IMPLEMENTED_NIDS.contains(&0x744680a2));
    }

    #[test]
    fn parse_retail_eboot_imports() {
        // Exercises the import parser against a real retail EBOOT
        // (the fixture under tools/rpcs3/dev_hdd0/). Any PS3 game
        // binary will work; the assertions pin this specific fixture
        // so the test fails if the import layout is misparsed.
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

        // Spot-check three modules nearly every retail PS3 game imports:
        // the sysutil helper library, the sysPrxForUser CRT layer, and
        // cellGcmSys for the RSX command-buffer setup.
        let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"cellSysutil"));
        assert!(names.contains(&"sysPrxForUser"));
        assert!(names.contains(&"cellGcmSys"));
    }

    #[test]
    fn no_prx_param_returns_error() {
        // A minimal ELF with no PT_0x60000002.
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        data[4] = 2; // 64-bit
        data[5] = 2; // big-endian
                     // phoff = 64, phentsize = 56, phnum = 0
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

        // Each trampoline is 24 bytes: OPD (8) + code (16).
        // Verify each trampoline's code portion.
        for (i, binding) in bindings.iter().enumerate() {
            let base = (tramp_base + (i as u32) * TRAMPOLINE_SIZE) as usize;
            // OPD at base: code_addr should point to base + 8
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

            // Code at base + 8: lis r11, hi; ori r11, r11, lo; sc; blr
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

        // Verify the last binding's GOT entry was patched to point
        // to the trampoline OPD.
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
