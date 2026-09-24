//! Loading and binding a firmware set, and the trampoline-only path when none loads.

use std::collections::BTreeMap;

use cellgov_mem::GuestMemory;

use crate::prx::got::patch_got_atomic;
use crate::prx::types::{HostLinkMaps, PrxLoadInfo, UnresolvedRequesters, VerifiedFirmware};
use crate::{BootSink, KeyVaultSource};

use super::base::{page_align_up_u64, resolve_prx_base};
use super::discover::{verify_against_manifest, FirmwareCandidates};
use super::error::FirmwareLoadError;

/// Install unresolved-import trampolines for every game import when
/// no firmware was loaded. Returns a synthetic [`PrxLoadInfo`]
/// describing the trampoline region so the alloc-base computation
/// accounts for it (`None` when the game has no imports), plus the
/// trampolined-NID requester map for the host diagnostic.
///
/// # Errors
///
/// [`FirmwareLoadError::TrampolineGotPatch`] when the one batch this
/// path stages is refused.
pub fn install_unresolved_trampolines_only(
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    tramp_base: u64,
    sink: &dyn BootSink,
) -> Result<(Option<PrxLoadInfo>, UnresolvedRequesters), FirmwareLoadError> {
    let stats = patch_got_atomic(modules, mem, tramp_base, |_, _| None)
        .map_err(|source| FirmwareLoadError::TrampolineGotPatch { source })?;
    if stats.variables_unbound > 0 {
        sink.warn(&format!(
            "prx: {} variable import(s) left unbound (no variable-import binder; \
             each vref slot keeps its pre-load bytes)",
            stats.variables_unbound,
        ));
    }
    if stats.trampolined == 0 {
        return Ok((None, std::collections::BTreeMap::new()));
    }
    sink.note(&format!(
        "prx: no firmware loaded -- {} game imports routed to unresolved-import trampoline \
         (region 0x{tramp_base:08x}..0x{:08x})",
        stats.trampolined, stats.tramp_region_end,
    ));
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
    Ok((Some(info), stats.unresolved_requesters))
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
/// directory was supplied.
///
/// `prx_base` places the set there instead of at the first 64K page
/// past `code_floor`.
///
/// # Errors
///
/// Any refusal of the scan, the selection, the placement or the GOT
/// batch; see [`FirmwareLoadError`].
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is an independent input of the one load; a bag would be built for this caller alone"
)]
pub fn load_firmware_set_bound(
    firmware_dir: Option<&str>,
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    code_floor: u32,
    prx_base: Option<u64>,
    include_internal: bool,
    sink: &dyn BootSink,
    keys: &dyn KeyVaultSource,
) -> Result<(Vec<PrxLoadInfo>, Option<VerifiedFirmware>, HostLinkMaps), FirmwareLoadError> {
    let Some(dir) = firmware_dir else {
        sink.note("prx: firmware-set mode requires --firmware-dir");
        // The trampolines-only fallback places from the code floor, so
        // the run identity names an override this boot never applied.
        if let Some(base) = prx_base {
            sink.warn(&format!(
                "prx: boot override prx_base=0x{base:x} set, but no firmware set is loaded -- \
                 it has no effect"
            ));
        }
        return Ok((Vec::new(), None, HostLinkMaps::default()));
    };
    let candidates = FirmwareCandidates::scan(dir, include_internal, keys)?;
    let (loaded, identity, host_link) =
        load_firmware_set_from(&candidates, modules, mem, code_floor, prx_base, sink)?;
    Ok((loaded, Some(identity), host_link))
}

/// [`load_firmware_set_bound`] over an already-scanned universe, into
/// any address space: the boot's, or a spawned child's. Selection
/// runs against `modules`, the image's own import table, unless the
/// universe was scanned for a firmware executable.
///
/// # Errors
///
/// Any refusal of the selection, the placement or the GOT batch; see
/// [`FirmwareLoadError`].
pub fn load_firmware_set_from(
    candidates: &FirmwareCandidates,
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    code_floor: u32,
    prx_base: Option<u64>,
    sink: &dyn BootSink,
) -> Result<(Vec<PrxLoadInfo>, VerifiedFirmware, HostLinkMaps), FirmwareLoadError> {
    let fw_root = &candidates.root;
    let fw_manifest = &candidates.manifest;

    // A game names its roots in its own import table; a firmware
    // executable builds its import tables at runtime and names none,
    // so its load set is every viable candidate.
    let root_namespaces: Option<std::collections::BTreeSet<String>> = if candidates.include_internal
    {
        None
    } else {
        Some(modules.iter().map(|m| m.name.clone()).collect())
    };
    let selection = cellgov_ppu::prx_loader::select_import_closure(
        &candidates.modules,
        root_namespaces.as_ref(),
    )
    .map_err(|source| FirmwareLoadError::Selection { source })?;
    sink.note(&format!(
        "prx: import-closure selection: {} of {} candidate module(s) selected",
        selection.selected.len(),
        candidates.modules.len(),
    ));
    for (path, reason) in &selection.pruned {
        sink.note(&format!("prx: pruned {path}: {reason}"));
    }
    for ns in &selection.unprovided_roots {
        sink.note(&format!(
            "prx: title imports namespace {ns:?}: no firmware module provides it"
        ));
    }
    // A stem present but dropped by selection leaves the shell's
    // runtime load-by-path unbacked, so it is fatal like an absent
    // stem.
    for (stem, path) in &candidates.internal_paths {
        if selection.selected.contains(path) {
            continue;
        }
        let reason = selection
            .pruned
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, r)| r.to_string())
            .unwrap_or_else(|| "viable but not selected".to_string());
        return Err(FirmwareLoadError::InternalStemDropped { stem, reason });
    }

    // id_to_stem feeds the boot-side Lv2Host PRX registry so
    // firmware-side `_sys_prx_load_module(path)` can resolve guest
    // paths back to a kernel id (the registry is keyed by stem since
    // cellSysmoduleLoadModule passes guest paths).
    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut id_to_stem: BTreeMap<cellgov_ppu::prx_loader::PrxModuleId, String> = BTreeMap::new();
    for path_str in &selection.selected {
        let elf = candidates
            .modules
            .get(path_str)
            .expect("invariant: selection only returns candidate paths")
            .clone();
        let path = std::path::Path::new(path_str);
        verify_against_manifest(fw_manifest, fw_root, path, &elf)?;
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return Err(FirmwareLoadError::NoStem {
                    path: path_str.clone(),
                })
            }
        };
        let parsed = cellgov_ppu::sprx::parse_prx(&elf).map_err(|source| {
            FirmwareLoadError::ParseModule {
                path: path_str.clone(),
                source,
            }
        })?;
        id_to_stem.insert(parsed.module_id, stem);
        bytes_by_path.insert(path_str.clone(), elf);
    }

    let prx_base = resolve_prx_base(prx_base, code_floor)?;

    let image = cellgov_ppu::prx_loader::load_firmware_set(bytes_by_path, mem, prx_base).map_err(
        |source| FirmwareLoadError::LoadSet {
            base: prx_base,
            source,
        },
    )?;

    // Empty image (selection chose no module): fall back to prx_base,
    // not 0, so trampolines never land in the null page where a call
    // through a null OPD would reach them.
    let prx_high_water = image
        .loaded
        .values()
        .map(cellgov_ppu::sprx::LoadedPrx::resident_end)
        .max()
        .unwrap_or(prx_base);
    let tramp_base = page_align_up_u64(prx_high_water)?;
    let stats = patch_got_atomic(modules, mem, tramp_base, |ns, nid| {
        image.export_table.get(ns, nid)
    })
    .map_err(|source| FirmwareLoadError::GotPatch { source })?;
    sink.note(&format!(
        "prx: firmware-set loaded {} module(s), {} (namespace, NID) pairs in export table, \
         {}/{} game imports resolved to firmware OPDs, \
         {} routed to unresolved-import trampoline (region 0x{tramp_base:08x}..0x{:08x})",
        image.loaded.len(),
        image.export_table.len(),
        stats.resolved,
        stats.total,
        stats.trampolined,
        stats.tramp_region_end,
    ));
    if stats.variables_unbound > 0 {
        sink.warn(&format!(
            "prx: {} variable import(s) left unbound (no variable-import binder; \
             each vref slot keeps its pre-load bytes)",
            stats.variables_unbound,
        ));
    }
    // The losing module's callers resolve to the winner's
    // implementation.
    for (namespace, first, second) in &image.shadowed_export_libraries {
        sink.note(&format!(
            "prx: export namespace {namespace:?} published by {first:?} and {second:?}; \
             kept {first:?}, dropped the later library"
        ));
    }

    // Pure-data library -> NID -> OPD view for the sc 484 CoreOS
    // manual link; the host cannot reach the loader's export table
    // itself. Same key as the table -- the arm reads each guest
    // import entry's library-name pointer and resolves under it.
    let mut exports: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>> =
        std::collections::BTreeMap::new();
    for (ns, nid) in image.export_table.keys() {
        let opd =
            image
                .export_table
                .get(ns, nid)
                .ok_or_else(|| FirmwareLoadError::ExportVanished {
                    namespace: ns.to_string(),
                    nid,
                })?;
        let opd = u32::try_from(opd).map_err(|_| FirmwareLoadError::ExportBeyondU32 {
            namespace: ns.to_string(),
            nid,
            opd,
        })?;
        exports.entry(ns.to_string()).or_default().insert(nid, opd);
    }
    let host_link = HostLinkMaps {
        exports,
        unresolved_requesters: stats.unresolved_requesters,
    };

    let mut out: Vec<PrxLoadInfo> = Vec::with_capacity(image.loaded.len());
    // Park the trampoline region as a synthetic PrxLoadInfo entry so
    // the alloc_base computation accounts for it via
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
            return Err(FirmwareLoadError::OrderWithoutModule { id: id.0 });
        };
        // `load_firmware_set` keys `loaded` by the same
        // `parse_prx().module_id` this map was built with, and
        // rejects a duplicate id outright, so a miss is a broken
        // loader invariant.
        let stem =
            id_to_stem
                .get(id)
                .cloned()
                .ok_or_else(|| FirmwareLoadError::ModuleWithoutStem {
                    id: id.0,
                    name: prx.name.clone(),
                })?;
        out.push(PrxLoadInfo {
            name: prx.name.clone(),
            stem,
            base: prx.base,
            data_end: prx.resident_end(),
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
    Ok((out, identity, host_link))
}
