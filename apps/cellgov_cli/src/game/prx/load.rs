//! Load firmware PRX(es) and bind imports through
//! [`super::got::patch_got_atomic`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_mem::GuestMemory;

use crate::cli::exit::die;

use super::got::patch_got_atomic;
use super::types::{HostLinkMaps, PrxLoadInfo, PrxLoadStageError, VerifiedFirmware};

use cellgov_ppu::prx_loader::FIRMWARE_INTERNAL_PRX_STEMS;

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

/// Every `*.sprx` directly under `dir_path`, sorted; an unreadable
/// directory or entry dies with the io error.
fn scan_sprx_files(dir_path: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir_path)
        .unwrap_or_else(|e| die(&format!("prx: read_dir {}: {e}", dir_path.display())));
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| {
            die(&format!(
                "prx: read_dir entry under {}: {e}",
                dir_path.display()
            ))
        });
        let p = entry.path();
        if p.extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        {
            paths.push(p);
        }
    }
    paths.sort();
    paths
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
    // A firmware tree carries no NPDRM content and no RAP to resolve
    // against, so its SELFs are APP-keyed by construction.
    cellgov_install::self_image::into_plaintext_elf(
        raw,
        cellgov_install::self_image::KeyPolicy::AppOnly,
    )
    .map_err(|source| PrxLoadStageError::Decrypt {
        path: path.to_path_buf(),
        source,
    })
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
/// computation accounts for it (`None` when the game has no imports),
/// plus the trampolined-NID requester map for the host diagnostic.
pub(in crate::game) fn install_unresolved_trampolines_only(
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    tramp_base: u64,
) -> (
    Option<PrxLoadInfo>,
    std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>,
) {
    let stats = match patch_got_atomic(modules, mem, tramp_base, |_, _| None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prx: trampoline-only GOT patch aborted ({e})");
            return (None, std::collections::BTreeMap::new());
        }
    };
    if stats.variables_unbound > 0 {
        eprintln!(
            "prx: {} variable import(s) left unbound (no variable-import binder; \
             each vref slot keeps its pre-load bytes)",
            stats.variables_unbound,
        );
    }
    if stats.trampolined == 0 {
        return (None, std::collections::BTreeMap::new());
    }
    println!(
        "prx: no firmware loaded -- {} game imports routed to unresolved-import trampoline \
         (region 0x{tramp_base:08x}..0x{:08x})",
        stats.trampolined, stats.tramp_region_end,
    );
    let info = PrxLoadInfo {
        name: "<unresolved-import-trampolines>".to_string(),
        stem: String::new(),
        base: tramp_base,
        data_end: stats.tramp_region_end,
        toc: 0,
        relocs_applied: 0,
        module_start: None,
        module_stop: None,
    };
    (Some(info), stats.unresolved_requesters)
}

/// Load the title's derived firmware set -- import-closure selection
/// over the candidate universe, then
/// [`cellgov_ppu::prx_loader::load_firmware_set`] -- patch the game
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
    include_internal: bool,
) -> (Vec<PrxLoadInfo>, Option<VerifiedFirmware>, HostLinkMaps) {
    let Some(dir) = firmware_dir else {
        println!("prx: firmware-set mode requires --firmware-dir");
        return (Vec::new(), None, HostLinkMaps::default());
    };
    let dir_path = std::path::PathBuf::from(dir);
    let (fw_root, fw_manifest) = locate_and_parse_manifest(&dir_path);

    // Candidate universe: every module in the firmware directory,
    // plus -- for a firmware executable -- the sys/internal stems the
    // shell loads by path at runtime. Their imports participate in
    // viability like anyone else's.
    let mut candidates: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for path in scan_sprx_files(&dir_path) {
        let elf = match read_firmware_module_elf(&path) {
            Ok(d) => d,
            Err(e) => die(&format!("prx: {e}")),
        };
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => die(&format!("prx: non-utf8 firmware path: {}", path.display())),
        };
        candidates.insert(path_str, elf);
    }
    if candidates.is_empty() {
        die(&format!(
            "prx: firmware-set mode: no .sprx modules under {}",
            dir_path.display()
        ));
    }
    let mut internal_paths: Vec<(&str, String)> = Vec::new();
    if include_internal {
        let internal_dir = fw_root.join("sys").join("internal");
        // The shell loads internal modules by guest path at runtime
        // (sc 480), so the whole directory is candidate material;
        // viability prunes what does not close, with the reason
        // printed. A prune is fatal only for the
        // FIRMWARE_INTERNAL_PRX_STEMS entries, which the shell cannot
        // boot without.
        for path in scan_sprx_files(&internal_dir) {
            let elf = match read_firmware_module_elf(&path) {
                Ok(d) => d,
                Err(e) => die(&format!("prx: {e}")),
            };
            let path_str = match path.to_str() {
                Some(s) => s.to_string(),
                None => die(&format!("prx: non-utf8 firmware path: {}", path.display())),
            };
            candidates.insert(path_str, elf);
        }
        for stem in FIRMWARE_INTERNAL_PRX_STEMS {
            let path = find_firmware_module(&internal_dir, stem).unwrap_or_else(|| {
                die(&format!(
                    "prx: firmware-exec boot needs sys/internal/{stem}, absent under {}",
                    internal_dir.display()
                ))
            });
            let path_str = match path.to_str() {
                Some(s) => s.to_string(),
                None => die(&format!("prx: non-utf8 firmware path: {}", path.display())),
            };
            // The directory scan covers .sprx; a pre-decrypted .prx
            // resolved by the stem walk still needs its bytes read.
            if !candidates.contains_key(&path_str) {
                let elf = match read_firmware_module_elf(&path) {
                    Ok(d) => d,
                    Err(e) => die(&format!("prx: {e}")),
                };
                candidates.insert(path_str.clone(), elf);
            }
            internal_paths.push((stem, path_str));
        }
    }

    // A game names its roots in its own import table; a firmware
    // executable builds its import tables at runtime and names none,
    // so its load set is every viable candidate.
    let root_namespaces: Option<std::collections::BTreeSet<String>> = if include_internal {
        None
    } else {
        Some(modules.iter().map(|m| m.name.clone()).collect())
    };
    let selection =
        match cellgov_ppu::prx_loader::select_import_closure(&candidates, root_namespaces.as_ref())
        {
            Ok(s) => s,
            Err(e) => die(&format!("prx: firmware-set selection failed: {e}")),
        };
    println!(
        "prx: import-closure selection: {} of {} candidate module(s) selected",
        selection.selected.len(),
        candidates.len(),
    );
    for (path, reason) in &selection.pruned {
        println!("prx: pruned {path}: {reason}");
    }
    for ns in &selection.unprovided_roots {
        println!("prx: title imports namespace {ns:?}: no firmware module provides it");
    }
    // A stem present but dropped by selection leaves the shell's
    // runtime load-by-path unbacked, so it is fatal like an absent
    // stem.
    for (stem, path) in &internal_paths {
        if selection.selected.contains(path) {
            continue;
        }
        let reason = selection
            .pruned
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, r)| r.to_string())
            .unwrap_or_else(|| "viable but not selected".to_string());
        die(&format!(
            "prx: firmware-exec boot needs sys/internal/{stem}, but selection dropped it: {reason}"
        ));
    }

    // id_to_stem feeds the boot-side Lv2Host PRX registry so
    // firmware-side `_sys_prx_load_module(path)` can resolve guest
    // paths back to a kernel id (the registry is keyed by stem since
    // cellSysmoduleLoadModule passes guest paths).
    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut id_to_stem: BTreeMap<cellgov_ppu::prx_loader::PrxModuleId, String> = BTreeMap::new();
    for path_str in &selection.selected {
        let elf = candidates
            .remove(path_str)
            .expect("selection only returns candidate paths");
        let path = std::path::Path::new(path_str);
        verify_against_manifest(&fw_manifest, &fw_root, path, &elf);
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => die(&format!("prx: cannot derive a module stem from {path_str}")),
        };
        match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(parsed) => {
                id_to_stem.insert(parsed.module_id, stem);
            }
            Err(e) => die(&format!("prx: failed to parse {path_str}: {e:?}")),
        }
        bytes_by_path.insert(path_str.clone(), elf);
    }

    let prx_base = resolve_prx_base(code_floor);

    let image = match cellgov_ppu::prx_loader::load_firmware_set(bytes_by_path, mem, prx_base) {
        Ok(img) => img,
        Err(e) => die(&format!(
            "prx: firmware-set load failed at base 0x{prx_base:x}: {e:?}"
        )),
    };

    // Empty image (selection chose no module): fall back to prx_base,
    // not 0, so trampolines never land in the null page where a call
    // through a null OPD would reach them.
    let prx_high_water = image
        .loaded
        .values()
        .map(|p| p.data_end)
        .max()
        .unwrap_or(prx_base);
    let tramp_base = page_align_up_u64(prx_high_water);
    let stats = match patch_got_atomic(modules, mem, tramp_base, |ns, nid| {
        image.export_table.get(ns, nid)
    }) {
        Ok(s) => s,
        Err(e) => die(&format!("prx: firmware-set GOT patch aborted ({e})")),
    };
    println!(
        "prx: firmware-set loaded {} module(s), {} (namespace, NID) pairs in export table, \
         {}/{} game imports resolved to firmware OPDs, \
         {} routed to unresolved-import trampoline (region 0x{tramp_base:08x}..0x{:08x})",
        image.loaded.len(),
        image.export_table.len(),
        stats.resolved,
        stats.total,
        stats.trampolined,
        stats.tramp_region_end,
    );
    if stats.variables_unbound > 0 {
        eprintln!(
            "prx: {} variable import(s) left unbound (no variable-import binder; \
             each vref slot keeps its pre-load bytes)",
            stats.variables_unbound,
        );
    }
    // The losing module's callers resolve to the winner's
    // implementation.
    for (namespace, first, second) in &image.shadowed_export_libraries {
        println!(
            "prx: export namespace {namespace:?} published by {first:?} and {second:?}; \
             kept {first:?}, dropped the later library"
        );
    }

    // Pure-data library -> NID -> OPD view for the sc 484 CoreOS
    // manual link; the host cannot reach the loader's export table
    // itself. Same key as the table -- the arm reads each guest
    // import entry's library-name pointer and resolves under it.
    let mut exports: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>> =
        std::collections::BTreeMap::new();
    for (ns, nid) in image.export_table.keys() {
        let opd = image.export_table.get(ns, nid).unwrap_or_else(|| {
            die(&format!(
                "prx: export table key {ns:?}::0x{nid:08x} vanished between keys() and get()"
            ))
        });
        let opd = u32::try_from(opd).unwrap_or_else(|_| {
            die(&format!(
                "prx: export {ns:?}::0x{nid:08x} OPD 0x{opd:x} exceeds the 32-bit \
                 guest address space"
            ))
        });
        exports.entry(ns.to_string()).or_default().insert(nid, opd);
    }
    let host_link = HostLinkMaps {
        exports,
        unresolved_requesters: stats.unresolved_requesters,
    };

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
            // `FirmwareImage::topological_order` is documented as a
            // permutation of `loaded.keys()`.
            die(&format!(
                "prx: topological order names module id 0x{:08x} absent from the \
                 loaded set; the loader's order/loaded invariant broke",
                id.0,
            ));
        };
        out.push(PrxLoadInfo {
            name: prx.name.clone(),
            // `load_firmware_set` keys `loaded` by the same
            // `parse_prx().module_id` this map was built with, and
            // rejects a duplicate id outright, so a miss is a broken
            // loader invariant.
            stem: id_to_stem.get(id).cloned().unwrap_or_else(|| {
                die(&format!(
                    "prx: loaded module id 0x{:08x} ({:?}) has no recorded stem; the \
                     loader's module-id/stem invariant broke",
                    id.0, prx.name,
                ))
            }),
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
    (out, Some(identity), host_link)
}
