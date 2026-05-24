//! Load firmware PRX(es) and bind imports through
//! [`super::got::patch_got_atomic`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_mem::GuestMemory;

use crate::cli::exit::die;

use super::got::patch_got_atomic;
use super::types::{PrxLoadInfo, PrxLoadStageError};

use cellgov_ppu::prx_loader::MIN_VIABLE_PRX_STEMS;

/// Locate the firmware module file for `stem` under `dir_path`.
///
/// Prefers `.sprx` (SCE-wrapped) over `.prx` (pre-decrypted) so both
/// boot modes converge on the same on-disk file when both exist.
fn find_firmware_module(dir_path: &Path, stem: &str) -> Option<PathBuf> {
    let sprx = dir_path.join(format!("{stem}.sprx"));
    if sprx.is_file() {
        return Some(sprx);
    }
    let prx = dir_path.join(format!("{stem}.prx"));
    if prx.is_file() {
        return Some(prx);
    }
    None
}

/// Read a firmware module file and decrypt if SCE-wrapped. Returns
/// the raw bytes otherwise so pre-decrypted `.prx` files load through
/// the same path.
fn read_firmware_module_elf(path: &Path) -> Result<Vec<u8>, PrxLoadStageError> {
    let raw = std::fs::read(path).map_err(|source| PrxLoadStageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.len() >= 4 && &raw[..4] == b"SCE\0" {
        cellgov_firmware::sce::decrypt_self_to_elf(&raw).map_err(|source| {
            PrxLoadStageError::Decrypt {
                path: path.to_path_buf(),
                source,
            }
        })
    } else {
        Ok(raw)
    }
}

/// Round `addr` up to the next 4 KiB boundary.
fn page_align_up_u64(addr: u64) -> u64 {
    addr.checked_add(0xFFF)
        .unwrap_or_else(|| die(&format!("page_align_up_u64: 0x{addr:x} + 0xFFF overflows")))
        & !0xFFFu64
}

/// Resolve the PRX placement base, honoring `CELLGOV_PRX_BASE` and
/// falling back to the first 64K-aligned page past `code_floor`.
/// Callers must set `code_floor` past every prior allocation in the
/// main region; this function does not validate that.
fn resolve_prx_base(code_floor: u32) -> u64 {
    let s = match std::env::var("CELLGOV_PRX_BASE") {
        Ok(s) => s,
        Err(_) => return (code_floor as u64 + 0xFFFF) & !0xFFFF,
    };
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let base = u64::from_str_radix(stripped, 16)
        .unwrap_or_else(|e| die(&format!("CELLGOV_PRX_BASE={s:?}: not a hex u64 ({e})")));
    if base & 0xFFFF != 0 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: must be 64K-aligned (low 16 bits zero)"
        ));
    }
    if base < code_floor as u64 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: below code_floor 0x{code_floor:x}"
        ));
    }
    // Main region spans `[0, 0x4000_0000)`; PRX placement above that
    // hits reserved or unmapped regions.
    if base >= 0x4000_0000 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: must be in main region (< 0x4000_0000)"
        ));
    }
    base
}

/// Install unresolved-import trampolines for every game import when
/// no firmware was loaded. Returns a synthetic [`PrxLoadInfo`]
/// describing the trampoline region so boot.rs's alloc-base
/// computation accounts for it, or `None` when the game has no
/// imports.
pub(in crate::game) fn install_unresolved_trampolines_only(
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    tramp_base: u64,
) -> Option<PrxLoadInfo> {
    let stats = match patch_got_atomic(modules, mem, tramp_base, |_| None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prx: trampoline-only GOT patch aborted ({e})");
            return None;
        }
    };
    if stats.trampolined == 0 {
        return None;
    }
    println!(
        "prx: no firmware loaded -- {} game imports routed to unresolved-import trampoline \
         (region 0x{tramp_base:08x}..0x{:08x})",
        stats.trampolined, stats.tramp_region_end,
    );
    Some(PrxLoadInfo {
        name: "<unresolved-import-trampolines>".to_string(),
        stem: String::new(),
        base: tramp_base,
        data_end: stats.tramp_region_end,
        toc: 0,
        relocs_applied: 0,
        module_start: None,
        module_stop: None,
    })
}

/// Load the minimum viable PRX set via
/// [`cellgov_ppu::prx_loader::load_firmware_set`], patch the game
/// ELF's GOT slots against the resulting union export table, and
/// return one [`PrxLoadInfo`] per module in topological order.
///
/// Returns an empty vector when the firmware directory is absent, a
/// required PRX stem is missing, or any decrypt / parse / load /
/// GOT-patch step fails.
pub(in crate::game) fn load_firmware_set_bound(
    firmware_dir: Option<&str>,
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    code_floor: u32,
) -> Vec<PrxLoadInfo> {
    let Some(dir) = firmware_dir else {
        println!("prx: firmware-set mode requires --firmware-dir");
        return Vec::new();
    };
    let dir_path = std::path::PathBuf::from(dir);

    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    // id_to_stem feeds the boot-side Lv2Host PRX registry so
    // firmware-side `_sys_prx_load_module(path)` can resolve guest
    // paths back to a kernel id.
    let mut id_to_stem: BTreeMap<cellgov_ppu::prx_loader::PrxModuleId, String> = BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for stem in MIN_VIABLE_PRX_STEMS {
        let path = match find_firmware_module(&dir_path, stem) {
            Some(p) => p,
            None => {
                missing.push(*stem);
                continue;
            }
        };
        let elf = match read_firmware_module_elf(&path) {
            Ok(d) => d,
            Err(e) => {
                println!("prx: {e}");
                return Vec::new();
            }
        };
        // Pull module_id up front so the post-load image.loaded map
        // can be keyed back to the file stem (the registry is keyed
        // by stem since cellSysmoduleLoadModule passes guest paths).
        match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(parsed) => {
                id_to_stem.insert(parsed.module_id, (*stem).to_string());
            }
            Err(e) => {
                println!("prx: failed to parse {}: {e:?}", path.display());
                return Vec::new();
            }
        }
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => {
                println!("prx: non-utf8 firmware path: {}", path.display());
                return Vec::new();
            }
        };
        bytes_by_path.insert(path_str, elf);
    }
    if !missing.is_empty() {
        println!(
            "prx: firmware-set mode: minimum viable PRX stems missing under {}: {missing:?}",
            dir_path.display()
        );
        return Vec::new();
    }

    let prx_base = resolve_prx_base(code_floor);

    let image = match cellgov_ppu::prx_loader::load_firmware_set(bytes_by_path, mem, prx_base) {
        Ok(img) => img,
        Err(e) => {
            println!("prx: firmware-set load failed at base 0x{prx_base:x}: {e:?}");
            return Vec::new();
        }
    };

    let prx_high_water = image.loaded.values().map(|p| p.data_end).max().unwrap_or(0);
    let tramp_base = page_align_up_u64(prx_high_water);
    let stats = match patch_got_atomic(modules, mem, tramp_base, |nid| image.export_table.get(nid))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prx: firmware-set GOT patch aborted ({e})");
            return Vec::new();
        }
    };
    println!(
        "prx: firmware-set loaded {} module(s), {} NIDs in export table, \
         {}/{} game imports resolved to firmware OPDs, \
         {} routed to unresolved-import trampoline (region 0x{tramp_base:08x}..0x{:08x})",
        image.loaded.len(),
        image.export_table.len(),
        stats.resolved,
        stats.total,
        stats.trampolined,
        stats.tramp_region_end,
    );

    let mut out: Vec<PrxLoadInfo> = Vec::with_capacity(image.loaded.len());
    // Park the trampoline region as a synthetic PrxLoadInfo entry so
    // boot.rs's alloc_base computation accounts for it via
    // `prx_region_end`.
    if stats.trampolined > 0 {
        out.push(PrxLoadInfo {
            name: "<unresolved-import-trampolines>".to_string(),
            stem: String::new(),
            base: tramp_base,
            data_end: stats.tramp_region_end,
            toc: 0,
            relocs_applied: 0,
            module_start: None,
            module_stop: None,
        });
    }
    for id in &image.topological_order {
        let Some(prx) = image.loaded.get(id) else {
            continue;
        };
        out.push(PrxLoadInfo {
            name: prx.name.clone(),
            stem: id_to_stem.get(id).cloned().unwrap_or_default(),
            base: prx.base,
            data_end: prx.data_end,
            toc: prx.toc,
            relocs_applied: prx.relocs_applied,
            module_start: prx.module_start,
            module_stop: prx.module_stop,
        });
    }
    out
}
