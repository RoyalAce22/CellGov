//! Load firmware PRX(es) and bind imports through
//! [`super::got::patch_got_atomic`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_mem::GuestMemory;

use super::got::patch_got_atomic;
use super::types::{
    HostLinkMaps, PrxLoadInfo, PrxLoadStageError, UnresolvedRequesters, VerifiedFirmware,
};
use crate::{BootSink, KeyVaultSource};

use cellgov_ps3_abi::format::dev_flash::FIRMWARE_INTERNAL_PRX_STEMS;

/// Why a firmware set did not load, or did not bind.
#[derive(Debug, thiserror::Error)]
pub enum FirmwareLoadError {
    /// The walk could not read a directory under the firmware root.
    #[error("prx: read_dir {}: {source}", dir.display())]
    ReadDir {
        /// The directory the walk could not read.
        dir: PathBuf,
        /// What the walk refused with.
        #[source]
        source: std::io::Error,
    },
    /// One entry of a firmware directory could not be read.
    #[error("prx: read_dir entry under {}: {source}", dir.display())]
    ReadDirEntry {
        /// The directory the entry sits in.
        dir: PathBuf,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// No `firmware.toml` covers the directory the boot names.
    #[error(
        "no firmware.toml at or above {}; the firmware corpus is unverifiable. \
         Reinstall with `cellgov firmware install`, which writes the manifest.",
        dir.display()
    )]
    NoManifest {
        /// The directory the walk started from.
        dir: PathBuf,
    },
    /// The manifest could not be read.
    #[error("read {}: {source}", path.display())]
    ManifestRead {
        /// The manifest the walk found.
        path: PathBuf,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// The manifest could not be parsed.
    #[error("{}: {source}", path.display())]
    ManifestParse {
        /// The manifest the walk found.
        path: PathBuf,
        /// The parser's own account of the refusal.
        #[source]
        source: Box<cellgov_install::manifest::ManifestError>,
    },
    /// A loaded module sits outside the root its manifest governs.
    #[error(
        "firmware module {} is outside the manifest root {}; the manifest-root walk and \
         the module path disagree",
        file.display(),
        root.display()
    )]
    ModuleOutsideRoot {
        /// The module the walk selected.
        file: PathBuf,
        /// The install root its manifest governs.
        root: PathBuf,
    },
    /// A loaded module has no `[[files]]` entry vouching for it.
    #[error(
        "{}: not listed in firmware.toml ({rel:?}); the corpus and its manifest disagree. \
         Reinstall with `cellgov firmware install`.",
        file.display()
    )]
    ModuleNotInManifest {
        /// The module with no entry.
        file: PathBuf,
        /// Its root-relative path, as the manifest would spell it.
        rel: String,
    },
    /// A loaded module's post-decrypt bytes do not match the manifest.
    #[error(
        "{}: post-decrypt SHA-256 mismatch against firmware.toml\n  \
         expected {expected}\n  actual   {actual}\n\
         The file does not match the installed PUP revision.",
        file.display()
    )]
    ModuleDigestMismatch {
        /// The module that did not match.
        file: PathBuf,
        /// The digest the manifest records, in hex.
        expected: String,
        /// The digest its post-decrypt bytes hash to, in hex.
        actual: String,
    },
    /// A module file could not be read or decrypted.
    #[error("prx: {0}")]
    Stage(#[from] PrxLoadStageError),
    /// The title's own import tables do not parse, so no namespace
    /// names a firmware module to select.
    #[error("imports: parse failed: {source}")]
    ImportParse {
        /// The parser's own account of the refusal.
        source: cellgov_ppu::prx::ImportParseError,
    },
    /// Rounding the guest heap floor up to a 64 KB page overflowed.
    #[error("alloc_floor=0x{alloc_floor:x} + 0xFFFF overflows usize")]
    AllocFloorOverflow {
        /// The floor the round-up started from.
        alloc_floor: usize,
    },
    /// A firmware path is not Unicode, so it cannot key the candidate
    /// set.
    #[error("prx: non-utf8 firmware path: {}", path.display())]
    NonUtf8Path {
        /// The path as the host spells it.
        path: PathBuf,
    },
    /// The firmware directory holds no module to select from.
    #[error("prx: firmware-set mode: no .sprx modules under {}", dir.display())]
    NoModules {
        /// The directory that held none.
        dir: PathBuf,
    },
    /// A `sys/internal` stem a firmware executable cannot boot without
    /// is absent.
    #[error("prx: firmware-exec boot needs sys/internal/{stem}, absent under {}", dir.display())]
    InternalStemAbsent {
        /// The stem the shell loads by path.
        stem: &'static str,
        /// The directory the stem walk covered.
        dir: PathBuf,
    },
    /// A `sys/internal` stem is present but import-closure selection
    /// dropped it, leaving the shell's load-by-path unbacked.
    #[error(
        "prx: firmware-exec boot needs sys/internal/{stem}, but selection dropped it: {reason}"
    )]
    InternalStemDropped {
        /// The stem the shell loads by path.
        stem: &'static str,
        /// Why selection dropped it, or that selection did not choose it.
        reason: String,
    },
    /// Import-closure selection refused the candidate set.
    #[error("prx: firmware-set selection failed: {source}")]
    Selection {
        /// The selector's own account of the refusal.
        #[source]
        source: cellgov_ppu::prx_loader::PrxLoaderError,
    },
    /// A selected module's path carries no usable filename stem.
    #[error("prx: cannot derive a module stem from {path}")]
    NoStem {
        /// The selected path with no filename stem.
        path: String,
    },
    /// A selected module failed to parse as a PRX.
    #[error("prx: failed to parse {path}: {source}")]
    ParseModule {
        /// The module that did not parse.
        path: String,
        /// The parser's own account of the refusal.
        source: cellgov_ppu::sprx::PrxParseError,
    },
    /// The `prx_base` boot override names no placement the main region
    /// can hold.
    #[error("--prx-base 0x{base:016x}: {reason}")]
    PrxBase {
        /// The base the override named.
        base: u64,
        /// Which rule it broke.
        reason: String,
    },
    /// Rounding the PRX placement base up to a page overflowed.
    #[error("page_align_up_u64: 0x{addr:016x} + 0xFFF overflows")]
    PageAlignOverflow {
        /// The address the round-up started from.
        addr: u64,
    },
    /// The loader could not place the selected set.
    #[error("prx: firmware-set load failed at base 0x{base:016x}: {source}")]
    LoadSet {
        /// The placement base the loader was given.
        base: u64,
        /// The loader's own account of the refusal.
        source: cellgov_ppu::prx_loader::PrxLoaderError,
    },
    /// The GOT batch was rejected, so no import was bound.
    #[error("prx: firmware-set GOT patch aborted ({source})")]
    GotPatch {
        /// Which staging step refused the batch.
        #[source]
        source: PrxLoadStageError,
    },
    /// The trampoline-only GOT batch was rejected, so every import slot
    /// still holds its pre-load bytes.
    #[error("prx: trampoline-only GOT patch aborted ({source})")]
    TrampolineGotPatch {
        /// Which staging step refused the batch.
        #[source]
        source: PrxLoadStageError,
    },
    /// The loader's export table answered `keys()` with a pair its
    /// `get()` does not hold.
    #[error("prx: export table key {namespace:?}::0x{nid:08x} vanished between keys() and get()")]
    ExportVanished {
        /// The export namespace the key named.
        namespace: String,
        /// The NID the key named.
        nid: u32,
    },
    /// An export OPD lies outside the 32-bit guest address space.
    #[error(
        "prx: export {namespace:?}::0x{nid:08x} OPD 0x{opd:016x} exceeds the 32-bit \
         guest address space"
    )]
    ExportBeyondU32 {
        /// The export namespace.
        namespace: String,
        /// The NID.
        nid: u32,
        /// The OPD address that did not fit.
        opd: u64,
    },
    /// The loader's topological order names a module absent from its
    /// loaded set.
    #[error(
        "prx: topological order names module id 0x{id:08x} absent from the loaded set; \
         the loader's order/loaded invariant broke"
    )]
    OrderWithoutModule {
        /// The module id the order named.
        id: u32,
    },
    /// A loaded module has no recorded filesystem stem.
    #[error(
        "prx: loaded module id 0x{id:08x} ({name:?}) has no recorded stem; the loader's \
         module-id/stem invariant broke"
    )]
    ModuleWithoutStem {
        /// The loaded module's id.
        id: u32,
        /// Its name from its PRX header.
        name: String,
    },
}

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

/// Every `*.sprx` directly under `dir_path`, sorted.
///
/// # Errors
///
/// The directory or one of its entries could not be read.
fn scan_sprx_files(dir_path: &Path) -> Result<Vec<PathBuf>, FirmwareLoadError> {
    let entries = std::fs::read_dir(dir_path).map_err(|source| FirmwareLoadError::ReadDir {
        dir: dir_path.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| FirmwareLoadError::ReadDirEntry {
            dir: dir_path.to_path_buf(),
            source,
        })?;
        let p = entry.path();
        if p.extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Locate and parse `firmware.toml` at or above `dir_path`.
///
/// The install writes the manifest at the install root while boots
/// point at `<root>/sys/external`, so the walk covers the directory
/// itself and two levels up. Returns the root the manifest governs.
///
/// # Errors
///
/// No manifest covers `dir_path`, or the one found does not read or
/// parse.
fn locate_and_parse_manifest(
    dir_path: &Path,
) -> Result<(PathBuf, cellgov_install::manifest::FirmwareManifest), FirmwareLoadError> {
    let mut root = dir_path.to_path_buf();
    for _ in 0..3 {
        let candidate = root.join(cellgov_install::manifest::MANIFEST_FILE);
        if candidate.is_file() {
            let text = std::fs::read_to_string(&candidate).map_err(|source| {
                FirmwareLoadError::ManifestRead {
                    path: candidate.clone(),
                    source,
                }
            })?;
            let manifest = cellgov_install::manifest::parse_manifest(&text).map_err(|source| {
                FirmwareLoadError::ManifestParse {
                    path: candidate.clone(),
                    source: Box::new(source),
                }
            })?;
            return Ok((root, manifest));
        }
        if !root.pop() {
            break;
        }
    }
    Err(FirmwareLoadError::NoManifest {
        dir: dir_path.to_path_buf(),
    })
}

/// Install-root-relative, forward-slash form of `file`, matching the
/// manifest's `[[files]].path` convention.
///
/// # Errors
///
/// `file` is not under `root`.
fn manifest_rel_path(root: &Path, file: &Path) -> Result<String, FirmwareLoadError> {
    let rel = file
        .strip_prefix(root)
        .map_err(|_| FirmwareLoadError::ModuleOutsideRoot {
            file: file.to_path_buf(),
            root: root.to_path_buf(),
        })?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Verify one loaded module's post-decrypt bytes against the manifest.
///
/// # Errors
///
/// The module is unlisted, or its digest does not match the one the
/// manifest records.
fn verify_against_manifest(
    manifest: &cellgov_install::manifest::FirmwareManifest,
    root: &Path,
    file: &Path,
    elf: &[u8],
) -> Result<(), FirmwareLoadError> {
    use cellgov_install::manifest::{sha256_of, verify_post_decrypt, Sha256, VerifyOutcome};
    let rel = manifest_rel_path(root, file)?;
    match verify_post_decrypt(manifest, &rel, &sha256_of(elf)) {
        VerifyOutcome::Match => Ok(()),
        VerifyOutcome::NotInManifest => Err(FirmwareLoadError::ModuleNotInManifest {
            file: file.to_path_buf(),
            rel,
        }),
        VerifyOutcome::Mismatch { expected, actual } => {
            Err(FirmwareLoadError::ModuleDigestMismatch {
                file: file.to_path_buf(),
                expected: Sha256(expected).to_hex(),
                actual: Sha256(actual).to_hex(),
            })
        }
    }
}

/// Read a firmware module file and decrypt if SCE-wrapped. Returns
/// the raw bytes otherwise so pre-decrypted `.prx` files load through
/// the same path.
fn read_firmware_module_elf(
    path: &Path,
    keys: &dyn KeyVaultSource,
) -> Result<Vec<u8>, PrxLoadStageError> {
    let raw = std::fs::read(path).map_err(|source| PrxLoadStageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    // A firmware tree carries no NPDRM content and no RAP to resolve
    // against, so its SELFs are APP-keyed by construction.
    let keys = keys
        .vault_for(&raw)
        .map_err(|e| PrxLoadStageError::KeyVault {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
    cellgov_install::self_image::into_plaintext_elf(
        raw,
        keys,
        cellgov_install::self_image::KeyPolicy::AppOnly,
    )
    .map_err(|source| PrxLoadStageError::Decrypt {
        path: path.to_path_buf(),
        source,
    })
}

/// Round `addr` up to the next 4 KiB boundary.
fn page_align_up_u64(addr: u64) -> Result<u64, FirmwareLoadError> {
    let rounded = addr
        .checked_add(0xFFF)
        .ok_or(FirmwareLoadError::PageAlignOverflow { addr })?;
    Ok(rounded & !0xFFFu64)
}

/// The PRX placement base: the `prx_base` boot override when the run
/// names one, [`default_prx_base`] otherwise.
///
/// # Errors
///
/// [`checked_prx_base`] refuses the override.
fn resolve_prx_base(prx_base: Option<u64>, code_floor: u32) -> Result<u64, FirmwareLoadError> {
    match prx_base {
        Some(base) => checked_prx_base(base, code_floor),
        None => Ok(default_prx_base(code_floor)),
    }
}

/// The first 64K-aligned page at or past `code_floor`.
///
/// Callers must set `code_floor` past every prior allocation in the
/// main region; this function does not validate that.
fn default_prx_base(code_floor: u32) -> u64 {
    (u64::from(code_floor) + 0xFFFF) & !0xFFFF
}

/// Check a `prx_base` override against the placements the main region can take.
///
/// # Errors
///
/// - `base` is not 64K-aligned.
/// - `base` is below `code_floor`.
/// - `base` is outside the main region.
fn checked_prx_base(base: u64, code_floor: u32) -> Result<u64, FirmwareLoadError> {
    let refuse = |reason: String| FirmwareLoadError::PrxBase { base, reason };
    if base & 0xFFFF != 0 {
        return Err(refuse("must be 64K-aligned (low 16 bits zero)".to_string()));
    }
    if base < code_floor as u64 {
        return Err(refuse(format!("below code_floor 0x{code_floor:x}")));
    }
    // Main region spans `[0, 0x4000_0000)`; PRX placement above that
    // hits reserved or unmapped regions.
    if base >= 0x4000_0000 {
        return Err(refuse("must be in main region (< 0x4000_0000)".to_string()));
    }
    Ok(base)
}

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

/// The firmware module universe one selection runs over: every
/// `.sprx` under the firmware directory (plus, for a firmware
/// executable, `sys/internal/`), decrypted once, with the manifest
/// that vouches for it.
pub struct FirmwareCandidates {
    root: PathBuf,
    manifest: cellgov_install::manifest::FirmwareManifest,
    /// Path -> post-decrypt ELF bytes.
    modules: BTreeMap<String, Vec<u8>>,
    /// `(stem, path)` of the sys/internal modules a firmware executable
    /// cannot boot without; empty unless `include_internal`.
    internal_paths: Vec<(&'static str, String)>,
    /// Whether the load set is every viable candidate (a firmware
    /// executable builds its import tables at runtime and names no
    /// roots) or the closure of the image's own import table.
    include_internal: bool,
}

impl FirmwareCandidates {
    /// Scan and decrypt `dir`.
    ///
    /// # Errors
    ///
    /// - A directory the scan walks does not read.
    /// - A module does not read or decrypt.
    /// - No `firmware.toml` covers the tree.
    /// - A stem a firmware executable needs is absent.
    pub fn scan(
        dir: &str,
        include_internal: bool,
        keys: &dyn KeyVaultSource,
    ) -> Result<Self, FirmwareLoadError> {
        let dir_path = std::path::PathBuf::from(dir);
        let (fw_root, fw_manifest) = locate_and_parse_manifest(&dir_path)?;

        // Candidate universe: every module in the firmware directory,
        // plus -- for a firmware executable -- the sys/internal stems the
        // shell loads by path at runtime. Their imports participate in
        // viability like anyone else's.
        let mut candidates: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for path in scan_sprx_files(&dir_path)? {
            let elf = read_firmware_module_elf(&path, keys)?;
            candidates.insert(utf8_path(&path)?, elf);
        }
        if candidates.is_empty() {
            return Err(FirmwareLoadError::NoModules { dir: dir_path });
        }
        let mut internal_paths: Vec<(&str, String)> = Vec::new();
        if include_internal {
            let internal_dir = fw_root.join("sys").join("internal");
            // The shell loads internal modules by guest path at runtime
            // (sc 480), so the whole directory is candidate material;
            // viability prunes what does not close, with the reason
            // reported. A prune is fatal only for the
            // FIRMWARE_INTERNAL_PRX_STEMS entries, which the shell cannot
            // boot without.
            for path in scan_sprx_files(&internal_dir)? {
                let elf = read_firmware_module_elf(&path, keys)?;
                candidates.insert(utf8_path(&path)?, elf);
            }
            for stem in FIRMWARE_INTERNAL_PRX_STEMS {
                let path = find_firmware_module(&internal_dir, stem).ok_or_else(|| {
                    FirmwareLoadError::InternalStemAbsent {
                        stem,
                        dir: internal_dir.clone(),
                    }
                })?;
                let path_str = utf8_path(&path)?;
                // The directory scan covers .sprx; a pre-decrypted .prx
                // resolved by the stem walk still needs its bytes read.
                if !candidates.contains_key(&path_str) {
                    let elf = read_firmware_module_elf(&path, keys)?;
                    candidates.insert(path_str.clone(), elf);
                }
                internal_paths.push((stem, path_str));
            }
        }
        Ok(Self {
            root: fw_root,
            manifest: fw_manifest,
            modules: candidates,
            internal_paths,
            include_internal,
        })
    }
}

/// The path as the candidate map keys it.
///
/// # Errors
///
/// The path is not Unicode.
fn utf8_path(path: &Path) -> Result<String, FirmwareLoadError> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| FirmwareLoadError::NonUtf8Path {
            path: path.to_path_buf(),
        })
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
        .map(|p| p.data_end)
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
    Ok((out, identity, host_link))
}

#[cfg(test)]
#[path = "tests/load_tests.rs"]
mod tests;
