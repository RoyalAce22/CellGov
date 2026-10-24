//! Loading and binding a firmware set, and the trampoline-only path when none loads.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_mem::GuestMemory;
use cellgov_ppu::prx_loader::{ClosureSelection, FirmwareExportTable, FirmwareImage, PrxModuleId};

use crate::prx::got::{patch_got_atomic, GotPatchStats};
use crate::prx::types::{HostLinkMaps, PrxLoadInfo, UnresolvedRequesters, VerifiedFirmware};
use crate::{BootSink, KeyVaultSource};

use super::base::{page_align_up_u64, resolve_prx_base};
use super::discover::{verify_against_manifest, FirmwareCandidates};
use super::error::FirmwareLoadError;

/// Each selected module's verified bytes, by candidate path.
type BytesByPath = BTreeMap<String, Vec<u8>>;

/// Each loaded module's file stem, by module id.
type StemById = BTreeMap<PrxModuleId, String>;

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
    let selection = select_and_report(candidates, modules, sink)?;
    let (bytes_by_path, id_to_stem) = read_selected(candidates, &selection)?;
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
    report_load(sink, &image, &stats, tramp_base);

    let tramp_region = (stats.trampolined > 0).then_some((tramp_base, stats.tramp_region_end));
    let host_link = HostLinkMaps {
        exports: host_link_exports(&image.export_table)?,
        unresolved_requesters: stats.unresolved_requesters,
    };
    let out = assemble_load_infos(&image, &id_to_stem, tramp_region)?;
    let identity = VerifiedFirmware {
        image_version: candidates.manifest.firmware.image_version.clone(),
        pup_sha256: candidates.manifest.firmware.pup_sha256.0,
    };
    Ok((out, identity, host_link))
}

/// Select the import closure to load from `candidates` and report the
/// selection, its prunes and the roots no module provides.
///
/// A game names its roots in its own import table; a firmware
/// executable builds its import tables at runtime and names none, so
/// its load set is every viable candidate. An internal stem that
/// selection dropped leaves the shell's runtime load-by-path unbacked,
/// so it is fatal like an absent stem.
fn select_and_report(
    candidates: &FirmwareCandidates,
    modules: &[cellgov_ppu::prx::ImportedModule],
    sink: &dyn BootSink,
) -> Result<ClosureSelection, FirmwareLoadError> {
    let root_namespaces: Option<BTreeSet<String>> = if candidates.include_internal {
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
    Ok(selection)
}

/// Read and verify each selected module. Returns the bytes by path and
/// the stem of each module id.
///
/// id_to_stem feeds the boot-side Lv2Host PRX registry so
/// firmware-side `_sys_prx_load_module(path)` can resolve guest
/// paths back to a kernel id (the registry keys by stem because
/// cellSysmoduleLoadModule passes guest paths).
fn read_selected(
    candidates: &FirmwareCandidates,
    selection: &ClosureSelection,
) -> Result<(BytesByPath, StemById), FirmwareLoadError> {
    let mut bytes_by_path = BytesByPath::new();
    let mut id_to_stem = StemById::new();
    for path_str in &selection.selected {
        let elf = candidates
            .modules
            .get(path_str)
            .expect("invariant: selection only returns candidate paths")
            .clone();
        let path = std::path::Path::new(path_str);
        verify_against_manifest(&candidates.manifest, &candidates.root, path, &elf)?;
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
    Ok((bytes_by_path, id_to_stem))
}

/// Report the loaded set, the variable imports left unbound, and each
/// export namespace two libraries published.
fn report_load(sink: &dyn BootSink, image: &FirmwareImage, stats: &GotPatchStats, tramp_base: u64) {
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
}

/// Return the pure-data library -> NID -> OPD view for the sc 484
/// CoreOS manual link; the host cannot reach the loader's export table
/// itself. Same key as the table -- the arm reads each guest import
/// entry's library-name pointer and resolves under it.
fn host_link_exports(
    export_table: &FirmwareExportTable,
) -> Result<BTreeMap<String, BTreeMap<u32, u32>>, FirmwareLoadError> {
    let mut exports: BTreeMap<String, BTreeMap<u32, u32>> = BTreeMap::new();
    for (ns, nid) in export_table.keys() {
        let opd = export_table
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
    Ok(exports)
}

/// Build one [`PrxLoadInfo`] per loaded module in topological order,
/// led by a synthetic entry for the trampoline region `(base, end)`
/// when the GOT patch routed any import there, so the alloc_base computation
/// accounts for it via `prx_region_end`.
fn assemble_load_infos(
    image: &FirmwareImage,
    id_to_stem: &StemById,
    tramp_region: Option<(u64, u64)>,
) -> Result<Vec<PrxLoadInfo>, FirmwareLoadError> {
    let mut out: Vec<PrxLoadInfo> = Vec::with_capacity(image.loaded.len());
    if let Some((tramp_base, tramp_region_end)) = tramp_region {
        out.push(PrxLoadInfo {
            name: "<unresolved-import-trampolines>".to_string(),
            stem: String::new(),
            base: tramp_base,
            data_end: tramp_region_end,
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
    Ok(out)
}
