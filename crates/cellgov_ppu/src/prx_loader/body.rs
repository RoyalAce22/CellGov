//! PRX loader implementation: errors, runner trait,
//! aggregate result, and the closure-loader driver.

use std::collections::{BTreeMap, BTreeSet};

use crate::prx_loader::export_table::FirmwareExportTable;
use crate::prx_loader::graph::{self, PrxModuleId};
use crate::sprx::{LoadedOpd, LoadedPrx};

/// Sentinel id attributing an `ImportTableParseFailed` to the game ELF.
/// Equal to FNV-1a-32 of the empty string; `parse_prx` rejects empty
/// module names, so no real module collides.
pub const SYNTHETIC_GAME_ELF_ID: PrxModuleId = PrxModuleId(0x811c_9dc5);

/// Namespaces accepted as dead-stub instead of rejecting the closure.
/// Their GOT slots keep pre-load values, so a guest call traps on the
/// unresolved stub.
const PERMITTED_MISSING_NAMESPACES: &[&str] = &["cellLibprof"];

fn is_permitted_missing(namespace: &str) -> bool {
    PERMITTED_MISSING_NAMESPACES.contains(&namespace)
}

/// Aggregate result of loading a firmware-PRX dependency closure.
#[derive(Debug)]
pub struct FirmwareImage {
    /// Per-module post-load state keyed by file-identity id.
    pub loaded: BTreeMap<PrxModuleId, LoadedPrx>,
    /// Merged NID -> OPD-address table across every loaded module.
    pub export_table: FirmwareExportTable,
    /// Dependency-respecting order used by [`start_modules`] and the
    /// import-patching pass; a permutation of `loaded.keys()`.
    pub topological_order: Vec<PrxModuleId>,
    /// Resolved import edges; self-imports filtered. Every target is
    /// a key in `loaded`.
    pub imports_by_id: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>>,
}

/// Failure surface for the multi-PRX loader.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrxLoaderError {
    /// Cycle in the import graph.
    #[error("cyclic PRX dependency (involves {} modules)", involved.len())]
    CyclicDependency {
        /// Modules participating in the strongly-connected component
        /// reported by Tarjan's algorithm; innocent downstream nodes
        /// are excluded.
        involved: Vec<PrxModuleId>,
    },
    /// A module named an import target whose `module_id` is not in
    /// the supplied byte set. Fires before [`Self::UnresolvedImport`]
    /// (which is the per-NID failure at GOT-patch time).
    #[error("PRX module {importer:?} imports missing target {target:?}")]
    MissingDependency {
        /// Module whose import table named the missing target.
        importer: PrxModuleId,
        /// Namespace id (not file id) that no loaded module publishes.
        target: PrxModuleId,
    },
    /// Two modules export the same NID with different OPD addresses.
    #[error("conflicting export NID 0x{nid:08x} between {first:?} and {second:?}")]
    ConflictingExport {
        /// NID with conflicting export definitions.
        nid: u32,
        /// First exporter encountered in topological order.
        first: PrxModuleId,
        /// Second exporter whose OPD address disagreed with `first`.
        second: PrxModuleId,
    },
    /// Game ELF / firmware module imports a NID no firmware module exports.
    #[error("unresolved import NID 0x{nid:08x}")]
    UnresolvedImport {
        /// NID that has no matching entry in the merged export table.
        nid: u32,
    },
    /// Per-import Phase-1 failure in `patch_imports_against`: a
    /// stub_addr did not form a valid `ByteRange`. No memory written.
    #[error("GOT patch failed at stub 0x{stub_addr:08x} for NID 0x{nid:08x}")]
    GotPatchFailed {
        /// Runtime GOT slot address that failed `ByteRange` validation.
        stub_addr: u32,
        /// NID whose stub slot triggered the failure.
        nid: u32,
    },
    /// Phase-2 batch failure: `StagingMemory::drain_into` rejected
    /// the resolved batch. Memory unchanged by the atomic-batch
    /// contract; item-level attribution is not available here.
    #[error("GOT batch patch ({count} writes) rejected: {source}")]
    GotBatchPatchFailed {
        /// Number of writes that were staged before the drain failed.
        count: usize,
        /// Underlying memory-layer error that caused the batch to abort.
        #[source]
        source: cellgov_mem::MemError,
    },
    /// Resolved OPD address did not fit in u32. PS3 LV2 user-space
    /// pointers are 32-bit by ABI; this signals a loader placement bug.
    #[error("OPD address 0x{addr:016x} for NID 0x{nid:08x} out of u32 range")]
    OpdAddressOutOfRange {
        /// NID whose resolved OPD address exceeded u32 range.
        nid: u32,
        /// Out-of-range OPD address as resolved from the export table.
        addr: u64,
    },
    /// Per-module load failed; wraps [`crate::sprx::PrxLoadError`].
    #[error("PRX load: {0}")]
    Load(#[source] crate::sprx::PrxLoadError),
    /// Per-module parse failed.
    #[error("PRX parse: {0}")]
    Parse(#[source] crate::sprx::PrxParseError),
    /// Per-module import-table parse failed. `NoImportsTable` is the
    /// legitimate no-imports-declared case and does not surface here.
    #[error("import-table parse for {module:?}: {source}")]
    ImportTableParseFailed {
        /// Module whose import-table parse failed; [`SYNTHETIC_GAME_ELF_ID`]
        /// attributes the failure to the game ELF.
        module: PrxModuleId,
        /// Underlying parser error from [`crate::prx::parse_imports`].
        #[source]
        source: crate::prx::ImportParseError,
    },
    /// `module_start` returned an error to the runner; `reason` is
    /// the runner-supplied payload.
    #[error("module_start for {module:?} failed: {reason}")]
    ModuleStartFailed {
        /// Module whose `module_start` invocation failed.
        module: PrxModuleId,
        /// Free-form failure payload supplied by the [`ModuleStartRunner`].
        reason: String,
    },
    /// A relocation referenced a segment index beyond `[text, data]`.
    /// `segment_idx` is the decoded segment number (>= 2).
    #[error("PRX {module:?} has multi-segment relocations (segment {segment_idx})")]
    MultiSegmentRelocations {
        /// Module whose relocation table referenced an unsupported segment.
        module: PrxModuleId,
        /// Decoded segment index (>= 2) that the per-module relocation
        /// applier cannot handle.
        segment_idx: usize,
    },
    /// Two paths in `bytes_by_path` produced the same `PrxModuleId`.
    #[error("duplicate PRX module id {id:?} in paths {first_path:?} and {second_path:?}")]
    DuplicateModuleId {
        /// File-identity id shared by both inputs.
        id: PrxModuleId,
        /// First path producing the id (lexicographically earliest key).
        first_path: String,
        /// Later path whose parse produced the same id.
        second_path: String,
    },
    /// Two PRXs publish the same export-namespace name.
    /// `namespace` is the hashed namespace id (see
    /// [`graph::module_id_from_name`]).
    #[error("duplicate export namespace {namespace:?} between {first:?} and {second:?}")]
    DuplicateExportNamespace {
        /// Hashed namespace id that both modules publish.
        namespace: PrxModuleId,
        /// First module observed to publish the namespace.
        first: PrxModuleId,
        /// Second module whose exports collide with `first`.
        second: PrxModuleId,
    },
    /// Cursor arithmetic overflowed u64 while laying out modules.
    #[error("load address space exhausted")]
    LoadAddressSpaceExhausted,
    /// `FirmwareExportTable::build` received an `order` slice with
    /// a duplicate `PrxModuleId`. Loader-internal bug.
    #[error(
        "duplicate module id {id:?} in order slice at indices {first_index} and {second_index}"
    )]
    DuplicateModuleInOrder {
        /// Module id appearing twice in the order slice.
        id: PrxModuleId,
        /// Index of the first occurrence in `order`.
        first_index: usize,
        /// Index of the duplicate occurrence in `order`.
        second_index: usize,
    },
    /// `FirmwareExportTable::build` received `loaded` and `order`
    /// whose key sets disagree. `order` must be a permutation of
    /// `loaded.keys()`.
    #[error(
        "order/loaded mismatch: {} only in order, {} only in loaded",
        in_order_not_loaded.len(),
        in_loaded_not_order.len()
    )]
    OrderLoadedMismatch {
        /// Ids present in `order` but absent from `loaded`.
        in_order_not_loaded: Vec<PrxModuleId>,
        /// Ids present in `loaded` but absent from `order`.
        in_loaded_not_order: Vec<PrxModuleId>,
    },
}

/// Implemented by callers that drive `module_start` execution.
pub trait ModuleStartRunner {
    /// Execute `module`'s `module_start` entry point described by `opd`.
    fn run_module_start(
        &mut self,
        module: &LoadedPrx,
        opd: LoadedOpd,
    ) -> Result<(), ModuleStartRunError>;
}

/// Runner-side failure surfaced through
/// [`PrxLoaderError::ModuleStartFailed`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModuleStartRunError {
    /// Runner-reported failure with a free-form reason.
    #[error("runner reported: {reason}")]
    RunnerReported {
        /// Free-form failure payload supplied by the runner; surfaced
        /// verbatim through [`PrxLoaderError::ModuleStartFailed`].
        reason: String,
    },
}

/// Decide whether `load_firmware_set` would accept `bytes` without
/// actually loading.
///
/// # Errors
///
/// - [`PrxLoaderError::Parse`] if the input is not a parseable PRX.
/// - [`PrxLoaderError::MultiSegmentRelocations`] if any relocation
///   references a segment beyond `[text, data]`.
pub fn check_loadable(bytes: &[u8]) -> Result<(), PrxLoaderError> {
    let parsed = crate::sprx::parse_prx(bytes).map_err(PrxLoaderError::Parse)?;
    check_relocations_within_text_data(&parsed)
}

fn check_relocations_within_text_data(
    parsed: &crate::sprx::ParsedPrx,
) -> Result<(), PrxLoaderError> {
    for r in &parsed.relocations {
        // PrxRelocation::sym encoding: low byte = target segment to
        // patch; next byte = value segment whose vaddr the addend is
        // relative to. The per-module relocation applier only knows
        // segments 0 (text) and 1 (data); >= 2 is the multi-segment
        // case.
        let target_seg = (r.sym & 0xFF) as usize;
        let value_seg = ((r.sym >> 8) & 0xFF) as usize;
        if target_seg >= 2 {
            return Err(PrxLoaderError::MultiSegmentRelocations {
                module: parsed.module_id,
                segment_idx: target_seg,
            });
        }
        if value_seg >= 2 {
            return Err(PrxLoaderError::MultiSegmentRelocations {
                module: parsed.module_id,
                segment_idx: value_seg,
            });
        }
    }
    Ok(())
}

/// Parse a module's imports; `NoImportsTable` becomes an empty list,
/// any other parse failure is a hard error.
fn parse_imports_or_propagate(
    bytes: &[u8],
    module: PrxModuleId,
) -> Result<Vec<crate::prx::ImportedModule>, PrxLoaderError> {
    match crate::prx::parse_imports(bytes) {
        Ok(v) => Ok(v),
        Err(crate::prx::ImportParseError::NoImportsTable) => Ok(Vec::new()),
        Err(source) => Err(PrxLoaderError::ImportTableParseFailed { module, source }),
    }
}

/// 64K-align upwards with overflow detection.
fn align_up_64k(cursor: u64) -> Result<u64, PrxLoaderError> {
    cursor
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFFu64)
        .ok_or(PrxLoaderError::LoadAddressSpaceExhausted)
}

/// Load every PRX in `bytes_by_path` in topological dependency order.
///
/// Each value must be a post-decrypt PRX ELF (SCE unwrapping is the
/// caller's responsibility). Modules are placed at successive
/// 64K-aligned bases starting at `base`; GOT slots for inter-firmware
/// imports are patched against the resulting export table.
///
/// # Errors
///
/// Any [`PrxLoaderError`] variant.
pub fn load_firmware_set(
    bytes_by_path: BTreeMap<String, Vec<u8>>,
    memory: &mut cellgov_mem::GuestMemory,
    base: u64,
) -> Result<FirmwareImage, PrxLoaderError> {
    let mut parsed_by_id: BTreeMap<PrxModuleId, (crate::sprx::ParsedPrx, Vec<u8>)> =
        BTreeMap::new();
    let mut path_by_id: BTreeMap<PrxModuleId, String> = BTreeMap::new();
    let mut imports_by_id: BTreeMap<PrxModuleId, Vec<crate::prx::ImportedModule>> = BTreeMap::new();
    // namespace_id (hash of an export-table module name like
    // "sysPrxForUser") -> the parsed module that publishes it. A
    // PRX's file-level identity (`parsed.module_id`) is distinct
    // from the names it exports under.
    let mut provider_of_namespace: BTreeMap<PrxModuleId, PrxModuleId> = BTreeMap::new();

    for (path, bytes) in &bytes_by_path {
        let parsed = crate::sprx::parse_prx(bytes).map_err(PrxLoaderError::Parse)?;
        let id = parsed.module_id;
        if let Some(first_path) = path_by_id.get(&id) {
            return Err(PrxLoaderError::DuplicateModuleId {
                id,
                first_path: first_path.clone(),
                second_path: path.clone(),
            });
        }
        check_relocations_within_text_data(&parsed)?;
        for lib in &parsed.exports {
            let ns_id = graph::module_id_from_name(&lib.name);
            if let Some(&first) = provider_of_namespace.get(&ns_id) {
                if first != id {
                    return Err(PrxLoaderError::DuplicateExportNamespace {
                        namespace: ns_id,
                        first,
                        second: id,
                    });
                }
            }
            provider_of_namespace.insert(ns_id, id);
        }
        let imports = parse_imports_or_propagate(bytes, id)?;
        imports_by_id.insert(id, imports);
        path_by_id.insert(id, path.clone());
        parsed_by_id.insert(id, (parsed, bytes.clone()));
    }

    // Translate each import's namespace name to the providing
    // module's id. A namespace with no provider in this closure
    // surfaces as MissingDependency unless it is on the
    // PERMITTED_MISSING_NAMESPACES allowlist. The manifest contract
    // names imports by namespace; resolving by NID alone would
    // rebind to any module exporting the same NID.
    let mut import_targets_by_id: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>> = BTreeMap::new();
    for (importer, imports) in &imports_by_id {
        let mut targets: BTreeSet<PrxModuleId> = BTreeSet::new();
        for imp in imports {
            let ns_id = graph::module_id_from_name(&imp.name);
            let provider = match provider_of_namespace.get(&ns_id) {
                Some(&p) => p,
                None if is_permitted_missing(&imp.name) => continue,
                None => {
                    return Err(PrxLoaderError::MissingDependency {
                        importer: *importer,
                        target: ns_id,
                    });
                }
            };
            if provider != *importer {
                targets.insert(provider);
            }
        }
        import_targets_by_id.insert(*importer, targets);
    }

    // Self-imports are dropped here so they never reach
    // topological_sort, which treats a self-edge as a cycle. The
    // `entry(*importer).or_default()` outside the inner loop
    // ensures every parsed module is a graph key even if it has
    // zero imports.
    let mut edges: BTreeMap<PrxModuleId, BTreeSet<PrxModuleId>> = BTreeMap::new();
    for (importer, targets) in &import_targets_by_id {
        edges.entry(*importer).or_default();
        for &target in targets {
            edges.entry(target).or_default().insert(*importer);
        }
    }

    let dep_graph = graph::topological_sort(&edges)?;

    let mut loaded: BTreeMap<PrxModuleId, LoadedPrx> = BTreeMap::new();
    let mut cursor = base;
    for id in &dep_graph.order {
        let Some((parsed, _)) = parsed_by_id.get(id) else {
            continue;
        };
        let aligned = align_up_64k(cursor)?;
        let l = crate::sprx::load_prx(parsed, memory, aligned).map_err(PrxLoaderError::Load)?;
        cursor = l.data_end;
        loaded.insert(*id, l);
    }

    let export_table = FirmwareExportTable::build(&loaded, &dep_graph.order)?;

    // Resolve any env-gated CELLGOV_HLE_RETURN_WATCH NIDs against the
    // freshly built firmware export table. No-op when the env var is
    // unset; otherwise registers (nid, entry_pc) so the per-instruction
    // dispatch hook can match.
    resolve_hle_watch_nids(&export_table, memory);

    for id in &dep_graph.order {
        let Some(imports) = imports_by_id.get(id) else {
            continue;
        };
        // Drop imports from permitted-missing namespaces so
        // patch_imports_against does not surface UnresolvedImport
        // for their NIDs; the matching dependency check above
        // already skipped these.
        let filtered: Vec<crate::prx::ImportedModule> = imports
            .iter()
            .filter(|m| !is_permitted_missing(&m.name))
            .cloned()
            .collect();
        // f.stub_addr from parse_imports is the link-time vaddr,
        // which for a PIC firmware PRX needs the runtime load base
        // added; passing `loaded[id].base` rebases each slot to its
        // post-load_prx address.
        let load_base = loaded
            .get(id)
            .map(|l| l.base)
            .expect("dep_graph.order id present in loaded");
        patch_imports_against(&filtered, &export_table, load_base, memory)?;
    }

    Ok(FirmwareImage {
        loaded,
        export_table,
        topological_order: dep_graph.order,
        imports_by_id: import_targets_by_id,
    })
}

/// Resolve env-gated [`crate::hle_watch`] NIDs to OPD entry PCs and
/// register them with the per-instruction dispatch hook; silent
/// no-op when the watch instrument is inactive.
#[allow(clippy::print_stderr)]
fn resolve_hle_watch_nids(export_table: &FirmwareExportTable, memory: &cellgov_mem::GuestMemory) {
    if !crate::hle_watch::is_active() {
        return;
    }
    for nid in crate::hle_watch::watched_nids() {
        let Some(opd_addr) = export_table.get(nid) else {
            eprintln!(
                "[cellgov] hle-return-watch: NID 0x{nid:08x} not present in firmware export table"
            );
            continue;
        };
        let range = match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(opd_addr), 4) {
            Some(r) => r,
            None => {
                eprintln!(
                    "[cellgov] hle-return-watch: NID 0x{nid:08x} OPD addr 0x{opd_addr:x} out of representable range"
                );
                continue;
            }
        };
        let entry_pc = match memory.read(range) {
            Some(bytes) => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            None => {
                eprintln!(
                    "[cellgov] hle-return-watch: NID 0x{nid:08x} OPD at 0x{opd_addr:x} not mapped"
                );
                continue;
            }
        };
        let name = cellgov_ps3_abi::nid::lookup(nid)
            .map(|(_, fname)| fname)
            .unwrap_or("<unknown>");
        crate::hle_watch::register_nid_resolution(nid, name, entry_pc);
    }
}

/// Patch the game ELF's import GOT slots against
/// `image.export_table`. Missing exports return
/// [`PrxLoaderError::UnresolvedImport`].
pub fn patch_game_imports(
    image: &FirmwareImage,
    game_elf: &[u8],
    memory: &mut cellgov_mem::GuestMemory,
) -> Result<(), PrxLoaderError> {
    let imports = parse_imports_or_propagate(game_elf, SYNTHETIC_GAME_ELF_ID)?;
    // PS3 retail games link at fixed vaddrs that match their
    // runtime placement, so the link-time stub_addrs are already
    // valid guest addresses.
    patch_imports_against(&imports, &image.export_table, 0, memory)
}

/// Walk one ELF's import table and patch each function's GOT slot
/// with the address of the exporting module's OPD.
///
/// `load_base` is the runtime base of the importing module's
/// segments. Pass `0` for game ELFs (link-time vaddr equals runtime
/// address); pass `LoadedPrx::base` for firmware PRXs loaded via
/// [`crate::sprx::load_prx`].
///
/// All writes drain as one batch via `StagingMemory::drain_into`; any
/// resolution failure leaves memory unmutated.
fn patch_imports_against(
    imports: &[crate::prx::ImportedModule],
    export_table: &FirmwareExportTable,
    load_base: u64,
    memory: &mut cellgov_mem::GuestMemory,
) -> Result<(), PrxLoaderError> {
    // Phase 1: resolve every (nid, stub_addr) without touching
    // memory or staging. Any failure here returns early with no
    // side effects.
    let mut resolved: Vec<(cellgov_mem::ByteRange, [u8; 4])> = Vec::new();
    for imp in imports {
        for f in &imp.functions {
            let Some(opd_addr) = export_table.get(f.nid) else {
                return Err(PrxLoaderError::UnresolvedImport { nid: f.nid });
            };
            let opd_u32 =
                u32::try_from(opd_addr).map_err(|_| PrxLoaderError::OpdAddressOutOfRange {
                    nid: f.nid,
                    addr: opd_addr,
                })?;
            let runtime_stub = load_base.checked_add(u64::from(f.stub_addr)).ok_or(
                PrxLoaderError::GotPatchFailed {
                    stub_addr: f.stub_addr,
                    nid: f.nid,
                },
            )?;
            let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(runtime_stub), 4)
                .ok_or(PrxLoaderError::GotPatchFailed {
                stub_addr: f.stub_addr,
                nid: f.nid,
            })?;
            resolved.push((range, opd_u32.to_be_bytes()));
        }
    }
    if resolved.is_empty() {
        return Ok(());
    }
    // Phase 2: stage every resolved write, then drain as one batch.
    // `StagingMemory::drain_into` validates against the region map
    // once and applies in stage order; a region-map failure leaves
    // both the buffer and `memory` untouched, preserving atomicity.
    let count = resolved.len();
    let mut staging = cellgov_mem::StagingMemory::new();
    for (range, bytes) in resolved {
        staging.stage(cellgov_mem::StagedWrite {
            range,
            bytes: bytes.to_vec(),
        });
    }
    match staging.drain_into(memory) {
        Ok(_) => Ok(()),
        Err(source) => {
            // Clear so the Drop debug-assert does not fire on the
            // rejected (unmutated) batch.
            staging.clear();
            Err(PrxLoaderError::GotBatchPatchFailed { count, source })
        }
    }
}

/// Invoke each module's `module_start` in `image.topological_order`.
/// Runner errors wrap into [`PrxLoaderError::ModuleStartFailed`].
pub fn start_modules<R: ModuleStartRunner>(
    image: &FirmwareImage,
    runner: &mut R,
) -> Result<(), PrxLoaderError> {
    for id in &image.topological_order {
        let Some(prx) = image.loaded.get(id) else {
            continue;
        };
        if let Some(opd) = prx.module_start {
            runner.run_module_start(prx, opd).map_err(|e| match e {
                ModuleStartRunError::RunnerReported { reason } => {
                    PrxLoaderError::ModuleStartFailed {
                        module: *id,
                        reason,
                    }
                }
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprx::LoadedOpd;

    fn stub_loaded(id: PrxModuleId, has_start: bool) -> LoadedPrx {
        LoadedPrx {
            name: format!("m{}", id.0),
            module_id: id,
            base: 0,
            toc: 0,
            text_start: 0,
            text_end: 0,
            data_start: 0,
            data_end: 0,
            exports: BTreeMap::new(),
            module_start: has_start.then_some(LoadedOpd {
                code: 0x1000 + u64::from(id.0),
                toc: 0,
            }),
            module_stop: None,
            relocs_applied: 0,
        }
    }

    struct Recorder {
        calls: Vec<PrxModuleId>,
    }

    impl ModuleStartRunner for Recorder {
        fn run_module_start(
            &mut self,
            module: &LoadedPrx,
            _opd: LoadedOpd,
        ) -> Result<(), ModuleStartRunError> {
            self.calls.push(module.module_id);
            Ok(())
        }
    }

    fn image_with_order(order: Vec<PrxModuleId>, with_start: &[PrxModuleId]) -> FirmwareImage {
        let loaded: BTreeMap<_, _> = order
            .iter()
            .map(|id| (*id, stub_loaded(*id, with_start.contains(id))))
            .collect();
        FirmwareImage {
            loaded,
            export_table: FirmwareExportTable::default(),
            topological_order: order,
            imports_by_id: BTreeMap::new(),
        }
    }

    #[test]
    fn start_modules_iterates_topological_order_field() {
        let order = vec![PrxModuleId(1), PrxModuleId(2), PrxModuleId(3)];
        let image = image_with_order(order.clone(), &order);
        let mut rec = Recorder { calls: Vec::new() };
        start_modules(&image, &mut rec).expect("start");
        assert_eq!(rec.calls, order);
    }

    #[test]
    fn start_modules_skips_modules_without_module_start() {
        let order = vec![PrxModuleId(1), PrxModuleId(2), PrxModuleId(3)];
        let image = image_with_order(order, &[PrxModuleId(2)]);
        let mut rec = Recorder { calls: Vec::new() };
        start_modules(&image, &mut rec).expect("start");
        assert_eq!(rec.calls, vec![PrxModuleId(2)]);
    }

    struct FailingRunner;
    impl ModuleStartRunner for FailingRunner {
        fn run_module_start(
            &mut self,
            module: &LoadedPrx,
            _opd: LoadedOpd,
        ) -> Result<(), ModuleStartRunError> {
            Err(ModuleStartRunError::RunnerReported {
                reason: format!("synthetic: {}", module.name),
            })
        }
    }

    fn stub_parsed(
        id: PrxModuleId,
        relocs: Vec<crate::sprx::PrxRelocation>,
    ) -> crate::sprx::ParsedPrx {
        crate::sprx::ParsedPrx {
            name: format!("synth-{}", id.0),
            module_id: id,
            toc: 0,
            text: crate::sprx::PrxSegment {
                vaddr: 0,
                filesz: 0,
                memsz: 0,
                data: Vec::new(),
            },
            data: crate::sprx::PrxSegment {
                vaddr: 0,
                filesz: 0,
                memsz: 0,
                data: Vec::new(),
            },
            exports: Vec::new(),
            relocations: relocs,
            module_start: None,
            module_stop: None,
        }
    }

    #[test]
    fn check_loadable_flags_relocation_into_third_segment() {
        let parsed = stub_parsed(
            PrxModuleId(7),
            vec![crate::sprx::PrxRelocation {
                offset: 0,
                rtype: 1,
                sym: 0x0203, // target_seg=3, value_seg=2
                addend: 0,
            }],
        );
        let err = check_relocations_within_text_data(&parsed).unwrap_err();
        assert_eq!(
            err,
            PrxLoaderError::MultiSegmentRelocations {
                module: PrxModuleId(7),
                segment_idx: 3,
            }
        );
    }

    #[test]
    fn check_loadable_flags_value_segment_alone_when_target_is_text() {
        // sym 0x0200 = target_seg=0 (text), value_seg=2 (out of range).
        let parsed = stub_parsed(
            PrxModuleId(9),
            vec![crate::sprx::PrxRelocation {
                offset: 0,
                rtype: 1,
                sym: 0x0200, // target_seg=0, value_seg=2
                addend: 0,
            }],
        );
        let err = check_relocations_within_text_data(&parsed).unwrap_err();
        assert_eq!(
            err,
            PrxLoaderError::MultiSegmentRelocations {
                module: PrxModuleId(9),
                segment_idx: 2,
            }
        );
    }

    #[test]
    fn check_loadable_accepts_text_and_data_only_relocations() {
        let parsed = stub_parsed(
            PrxModuleId(8),
            vec![
                crate::sprx::PrxRelocation {
                    offset: 0,
                    rtype: 1,
                    sym: 0x0000,
                    addend: 0,
                },
                crate::sprx::PrxRelocation {
                    offset: 0,
                    rtype: 1,
                    sym: 0x0101,
                    addend: 0,
                },
            ],
        );
        assert!(check_relocations_within_text_data(&parsed).is_ok());
    }

    #[test]
    fn start_modules_propagates_runner_error_with_reason_preserved() {
        let id = PrxModuleId(7);
        let image = image_with_order(vec![id], &[id]);
        let err = start_modules(&image, &mut FailingRunner).unwrap_err();
        assert_eq!(
            err,
            PrxLoaderError::ModuleStartFailed {
                module: id,
                reason: "synthetic: m7".to_string(),
            }
        );
    }

    #[test]
    fn synthetic_game_elf_id_equals_module_id_from_name_of_empty_string() {
        assert_eq!(SYNTHETIC_GAME_ELF_ID, graph::module_id_from_name(""));
    }

    #[test]
    fn module_id_from_name_is_stable_for_liblv2() {
        // sync_state_hash transitively depends on the FNV-1a-32
        // mapping for "liblv2" being byte-stable across runs and
        // hosts; drift here is a determinism regression.
        const EXPECTED: u32 = {
            const OFFSET: u32 = 0x811c_9dc5;
            const PRIME: u32 = 0x0100_0193;
            let bytes = b"liblv2";
            let mut h = OFFSET;
            let mut i = 0;
            while i < bytes.len() {
                h ^= bytes[i] as u32;
                h = h.wrapping_mul(PRIME);
                i += 1;
            }
            h
        };
        assert_eq!(graph::module_id_from_name("liblv2"), PrxModuleId(EXPECTED));
    }

    fn one_import(nid: u32, stub_addr: u32) -> Vec<crate::prx::ImportedModule> {
        vec![crate::prx::ImportedModule {
            name: "synth".to_string(),
            functions: vec![crate::prx::ImportedFunction { nid, stub_addr }],
            variables: Vec::new(),
        }]
    }

    #[test]
    fn patch_imports_against_unresolved_nid_yields_unresolved_import() {
        let table = FirmwareExportTable::default(); // empty
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        let err =
            patch_imports_against(&one_import(0xDEADBEEF, 0x100), &table, 0, &mut mem).unwrap_err();
        assert_eq!(err, PrxLoaderError::UnresolvedImport { nid: 0xDEADBEEF });
    }

    #[test]
    fn patch_imports_against_opd_above_u32_yields_out_of_range() {
        let table = FirmwareExportTable::for_test(&[(0xCAFEBABE, 0x1_0000_0000u64)]);
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        let err =
            patch_imports_against(&one_import(0xCAFEBABE, 0x100), &table, 0, &mut mem).unwrap_err();
        assert_eq!(
            err,
            PrxLoaderError::OpdAddressOutOfRange {
                nid: 0xCAFEBABE,
                addr: 0x1_0000_0000,
            }
        );
    }

    #[test]
    fn patch_imports_against_succeeds_and_writes_be_opd_into_got_slot() {
        let table = FirmwareExportTable::for_test(&[(0xAAAA1111, 0x4000_0080u64)]);
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        patch_imports_against(&one_import(0xAAAA1111, 0x100), &table, 0, &mut mem).expect("patch");
        let got = &mem.as_bytes()[0x100..0x104];
        assert_eq!(got, &0x4000_0080u32.to_be_bytes());
    }

    #[test]
    fn patch_imports_against_writes_at_load_base_plus_stub_addr() {
        // Firmware PRXs parse with PIC-base-0 vaddrs; patch fires at
        // `load_base + stub_addr`.
        let opd_addr: u64 = 0x4000_0080;
        let load_base: u64 = 0x2000;
        let stub_vaddr: u32 = 0x300;
        let runtime_stub = load_base + u64::from(stub_vaddr);
        let table = FirmwareExportTable::for_test(&[(0xAAAA1111, opd_addr)]);
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        patch_imports_against(
            &one_import(0xAAAA1111, stub_vaddr),
            &table,
            load_base,
            &mut mem,
        )
        .expect("patch");
        assert_eq!(
            &mem.as_bytes()[runtime_stub as usize..runtime_stub as usize + 4],
            &(opd_addr as u32).to_be_bytes(),
        );
        assert_eq!(
            &mem.as_bytes()[stub_vaddr as usize..stub_vaddr as usize + 4],
            &[0u8; 4],
            "load_base = 0x2000 should redirect the write away from vaddr 0x300"
        );
    }

    #[test]
    fn patch_imports_against_is_atomic_on_phase1_failure() {
        // First import resolves, second is missing; the failure must
        // discard the first.
        let table = FirmwareExportTable::for_test(&[(0xAAAA1111, 0x4000_0080u64)]);
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        let before = mem.content_hash();
        let imports = vec![crate::prx::ImportedModule {
            name: "synth".to_string(),
            functions: vec![
                crate::prx::ImportedFunction {
                    nid: 0xAAAA1111,
                    stub_addr: 0x100,
                },
                crate::prx::ImportedFunction {
                    nid: 0xBBBB2222,
                    stub_addr: 0x110,
                },
            ],
            variables: Vec::new(),
        }];
        let err = patch_imports_against(&imports, &table, 0, &mut mem).unwrap_err();
        assert_eq!(err, PrxLoaderError::UnresolvedImport { nid: 0xBBBB2222 });
        assert_eq!(
            mem.content_hash(),
            before,
            "Phase-1 failure committed bytes: atomic-batch violated"
        );
    }

    #[test]
    fn patch_imports_against_is_atomic_on_phase2_drain_failure() {
        // Both imports resolve in Phase-1; the second points outside
        // GuestMemory so the Phase-2 drain rejects the batch.
        let table = FirmwareExportTable::for_test(&[
            (0xAAAA1111, 0x4000_0080u64),
            (0xBBBB2222, 0x4000_00C0u64),
        ]);
        let mut mem = cellgov_mem::GuestMemory::new(0x1000);
        let before = mem.content_hash();
        let imports = vec![crate::prx::ImportedModule {
            name: "synth".to_string(),
            functions: vec![
                crate::prx::ImportedFunction {
                    nid: 0xAAAA1111,
                    stub_addr: 0x100,
                },
                crate::prx::ImportedFunction {
                    nid: 0xBBBB2222,
                    stub_addr: 0xFFFF_0000,
                },
            ],
            variables: Vec::new(),
        }];
        let err = patch_imports_against(&imports, &table, 0, &mut mem).unwrap_err();
        match err {
            PrxLoaderError::GotBatchPatchFailed { count, source: _ } => {
                assert_eq!(count, 2, "batch carries the full staged count");
            }
            other => panic!("expected GotBatchPatchFailed, got {other:?}"),
        }
        assert_eq!(
            mem.content_hash(),
            before,
            "Phase-2 drain failure committed bytes: atomic-batch violated"
        );
    }

    #[test]
    fn load_firmware_set_rejects_duplicate_module_id_and_does_not_touch_memory() {
        let bytes = crate::sprx::test_fixtures::make_test_prx();
        let mut by_path = BTreeMap::new();
        by_path.insert("alpha.sprx".to_string(), bytes.clone());
        by_path.insert("beta.sprx".to_string(), bytes);
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let before = mem.content_hash();
        let err = load_firmware_set(by_path, &mut mem, 0x1000_0000).unwrap_err();
        match err {
            PrxLoaderError::DuplicateModuleId {
                id: _,
                first_path,
                second_path,
            } => {
                // BTreeMap iteration is lexicographic by key: alpha
                // is first, beta is second.
                assert_eq!(first_path, "alpha.sprx");
                assert_eq!(second_path, "beta.sprx");
            }
            other => panic!("expected DuplicateModuleId, got {other:?}"),
        }
        assert_eq!(
            mem.content_hash(),
            before,
            "DuplicateModuleId fired but memory was mutated: dedup check must run before load_prx"
        );
    }

    #[test]
    fn patch_imports_against_empty_import_list_is_noop_ok() {
        let table = FirmwareExportTable::default();
        let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
        let before = mem.content_hash();
        patch_imports_against(&[], &table, 0, &mut mem).expect("patch");
        assert_eq!(mem.content_hash(), before);
    }

    // -- Export-namespace identity --

    /// Add a single synthetic import entry to `make_test_prx`'s bytes
    /// declaring an import of one NID from namespace `imp_name`. The
    /// entry is placed in segment 1 (data) past the existing layout
    /// at file offset 0x300; vaddr 0x210 in segment 1. The fixture's
    /// library_info imports_start/end and the import-table entry's
    /// name/nid/stub pointers are patched accordingly.
    fn make_test_prx_importing(imp_name: &str) -> Vec<u8> {
        let mut data = crate::sprx::test_fixtures::make_test_prx();
        // Entry at file 0x300 (vaddr 0x210); 0x2C bytes; one function.
        let entry_off: usize = 0x300;
        let imp_name_off: usize = entry_off + 0x30; // file 0x330, vaddr 0x240
        let imp_nid_off: usize = entry_off + 0x50; // file 0x350, vaddr 0x260
        let imp_stub_off: usize = entry_off + 0x60; // file 0x360, vaddr 0x270

        // library_info imports_start/end (file 0x1F0 + 44/48 = 0x21C/0x220):
        // entry begins at vaddr 0x210, ends at vaddr 0x210 + 0x2C = 0x23C.
        let mi = 0x1F0usize;
        data[mi + 44..mi + 48].copy_from_slice(&0x210u32.to_be_bytes());
        data[mi + 48..mi + 52].copy_from_slice(&0x23Cu32.to_be_bytes());

        // PrxImportEntry @ entry_off (vaddr 0x210):
        // size=0x2C, num_func=1, name_ptr/nid_ptr/stub_ptr.
        data[entry_off] = 0x2C;
        data[entry_off + 6..entry_off + 8].copy_from_slice(&1u16.to_be_bytes());
        // name_ptr (vaddr 0x240)
        data[entry_off + 16..entry_off + 20].copy_from_slice(&0x240u32.to_be_bytes());
        // nid_ptr (vaddr 0x260)
        data[entry_off + 20..entry_off + 24].copy_from_slice(&0x260u32.to_be_bytes());
        // stub_ptr (vaddr 0x270)
        data[entry_off + 24..entry_off + 28].copy_from_slice(&0x270u32.to_be_bytes());

        // Write the import-module name (NUL-terminated).
        let name_bytes = imp_name.as_bytes();
        assert!(
            name_bytes.len() < 32,
            "test fixture: name too long for 0x20-byte region"
        );
        data[imp_name_off..imp_name_off + name_bytes.len()].copy_from_slice(name_bytes);
        data[imp_name_off + name_bytes.len()] = 0;

        // One NID + stub slot.
        data[imp_nid_off..imp_nid_off + 4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        data[imp_stub_off..imp_stub_off + 4].copy_from_slice(&0u32.to_be_bytes());

        data
    }

    #[test]
    fn load_firmware_set_missing_namespace_reports_namespace_id() {
        let bytes = make_test_prx_importing("ghostlib");
        let mut by_path = BTreeMap::new();
        by_path.insert("solo.sprx".to_string(), bytes);
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let err = load_firmware_set(by_path, &mut mem, 0x1000_0000).unwrap_err();
        match err {
            PrxLoaderError::MissingDependency { target, .. } => {
                let expected = graph::module_id_from_name("ghostlib");
                assert_eq!(
                    target, expected,
                    "MissingDependency.target must be the namespace id, \
                     not the file's library_info-name id"
                );
            }
            other => panic!("expected MissingDependency, got {other:?}"),
        }
    }

    #[test]
    fn load_firmware_set_self_namespace_import_does_not_trip_missing_dependency() {
        // make_test_prx exports under "testlib"; importing that
        // namespace from itself must not trip MissingDependency.
        let bytes = make_test_prx_importing("testlib");
        let mut by_path = BTreeMap::new();
        by_path.insert("solo.sprx".to_string(), bytes);
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let result = load_firmware_set(by_path, &mut mem, 0x1000_0000);
        if let Err(PrxLoaderError::MissingDependency { target, .. }) = &result {
            panic!(
                "self-namespace import tripped MissingDependency (target={target:?}); \
                 expected the loader to recognise testlib's own export"
            );
        }
    }
}
