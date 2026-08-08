//! Load firmware PRX(es) and bind imports through
//! [`super::got::patch_got_atomic`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_mem::GuestMemory;

use crate::cli::exit::die;

use super::got::patch_got_atomic;
use super::types::{PrxLoadInfo, PrxLoadStageError, VerifiedFirmware};

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

/// Locate and parse `firmware.toml` at or above `dir_path`.
///
/// The install writes the manifest at the install root while boots
/// point at `<root>/sys/external`, so the walk covers the directory
/// itself and two levels up. Returns the root the manifest governs.
fn locate_and_parse_manifest(
    dir_path: &Path,
) -> (PathBuf, cellgov_install::manifest::FirmwareManifest) {
    let mut root = dir_path.to_path_buf();
    for _ in 0..3 {
        let candidate = root.join("firmware.toml");
        if candidate.is_file() {
            let text = std::fs::read_to_string(&candidate)
                .unwrap_or_else(|e| die(&format!("read {}: {e}", candidate.display())));
            let manifest = cellgov_install::manifest::parse_manifest(&text)
                .unwrap_or_else(|e| die(&format!("{}: {e}", candidate.display())));
            return (root, manifest);
        }
        if !root.pop() {
            break;
        }
    }
    die(&format!(
        "no firmware.toml at or above {}; the firmware corpus is unverifiable. \
         Reinstall with `cellgov_install install`, which writes the manifest.",
        dir_path.display()
    ))
}

/// Install-root-relative, forward-slash form of `file`, matching the
/// manifest's `[[files]].path` convention.
fn manifest_rel_path(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or_else(|_| {
        die(&format!(
            "firmware module {} is outside the manifest root {}; the \
             manifest-root walk and the module path disagree",
            file.display(),
            root.display()
        ))
    });
    rel.to_string_lossy().replace('\\', "/")
}

/// Verify one loaded module's post-decrypt bytes against the
/// manifest; any outcome but `Match` is a hard error.
fn verify_against_manifest(
    manifest: &cellgov_install::manifest::FirmwareManifest,
    root: &Path,
    file: &Path,
    elf: &[u8],
) {
    use cellgov_install::manifest::{sha256_of, verify_post_decrypt, Sha256, VerifyOutcome};
    let rel = manifest_rel_path(root, file);
    match verify_post_decrypt(manifest, &rel, &sha256_of(elf)) {
        VerifyOutcome::Match => {}
        VerifyOutcome::NotInManifest => die(&format!(
            "{}: not listed in firmware.toml ({rel:?}); the corpus and its \
             manifest disagree. Reinstall with `cellgov_install install`.",
            file.display()
        )),
        VerifyOutcome::Mismatch { expected, actual } => die(&format!(
            "{}: post-decrypt SHA-256 mismatch against firmware.toml\n  \
             expected {}\n  actual   {}\n\
             The file does not match the installed PUP revision.",
            file.display(),
            Sha256(expected).to_hex(),
            Sha256(actual).to_hex(),
        )),
    }
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
        cellgov_install::sce::decrypt_self_to_elf(&raw).map_err(|source| {
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
/// return one [`PrxLoadInfo`] per module in topological order plus
/// the manifest-verified firmware identity.
///
/// Every loaded module's post-decrypt bytes are checked against
/// `firmware.toml`; a missing manifest or a digest mismatch is a
/// hard error.
///
/// Returns an empty vector (and no identity) only when no firmware
/// directory was supplied; every other failure dies.
pub(in crate::game) fn load_firmware_set_bound(
    firmware_dir: Option<&str>,
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    code_floor: u32,
) -> (Vec<PrxLoadInfo>, Option<VerifiedFirmware>) {
    let Some(dir) = firmware_dir else {
        println!("prx: firmware-set mode requires --firmware-dir");
        return (Vec::new(), None);
    };
    let dir_path = std::path::PathBuf::from(dir);
    let (fw_root, fw_manifest) = locate_and_parse_manifest(&dir_path);

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
            Err(e) => die(&format!("prx: {e}")),
        };
        verify_against_manifest(&fw_manifest, &fw_root, &path, &elf);
        // Pull module_id up front so the post-load image.loaded map
        // can be keyed back to the file stem (the registry is keyed
        // by stem since cellSysmoduleLoadModule passes guest paths).
        match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(parsed) => {
                id_to_stem.insert(parsed.module_id, (*stem).to_string());
            }
            Err(e) => die(&format!("prx: failed to parse {}: {e:?}", path.display())),
        }
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => die(&format!("prx: non-utf8 firmware path: {}", path.display())),
        };
        bytes_by_path.insert(path_str, elf);
    }
    if !missing.is_empty() {
        die(&format!(
            "prx: firmware-set mode: minimum viable PRX stems missing under {}: {missing:?}",
            dir_path.display()
        ));
    }

    let prx_base = resolve_prx_base(code_floor);

    let image = match cellgov_ppu::prx_loader::load_firmware_set(bytes_by_path, mem, prx_base) {
        Ok(img) => img,
        Err(e) => die(&format!(
            "prx: firmware-set load failed at base 0x{prx_base:x}: {e:?}"
        )),
    };

    let prx_high_water = image.loaded.values().map(|p| p.data_end).max().unwrap_or(0);
    let tramp_base = page_align_up_u64(prx_high_water);
    let stats = match patch_got_atomic(modules, mem, tramp_base, |nid| image.export_table.get(nid))
    {
        Ok(s) => s,
        Err(e) => die(&format!("prx: firmware-set GOT patch aborted ({e})")),
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
    let identity = VerifiedFirmware {
        image_version: fw_manifest.firmware.image_version.clone(),
        pup_sha256: fw_manifest.firmware.pup_sha256.0,
    };
    (out, Some(identity))
}
