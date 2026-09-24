//! Emits firmware PPU syscall caller tables.
//!
//! `cellgov_install` opens each verified firmware module and
//! `cellgov_ppu` finds its syscall callers; this command maps those
//! callers into the archive's rows, merges and writes the tables.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::TitleRegistry;
use cellgov_install::firmware_verify::{
    self, FirmwareVerifyError, ModuleDivergence, ModuleFault, ModuleImage,
};
use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_lv2::archive::{
    self, ArchiveError, CallerCensus, CallerRow, CallerUnresolvedRow, ReachRow, CALLER,
    CALLER_UNRESOLVED, FIRMWARE, REACH,
};
use cellgov_ppu::caller_census::{
    module_callers, CallerScanError, ModuleCallers, ModuleCallersError,
};
use cellgov_ppu::funcmap::FuncMapError;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::CallerCensusArgs;
use crate::composition::refusal::firmware_refusal;
use cellgov_install::store::inventory::{FirmwareEntry, InventoryError, StoreInventory};
use cellgov_install::store::select::{select_firmware, FirmwareSelectError};

const FIRMWARE_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/firmware.tsv"
));

struct CensusTables {
    census: CallerCensus,
    modules: usize,
    resolved_sites: usize,
    unresolved_sites: usize,
}

#[derive(Debug, thiserror::Error)]
enum CallerCensusError {
    #[error("store inventory: {0}")]
    Inventory(#[from] InventoryError),
    #[error("{}", firmware_refusal(.0))]
    Select(#[from] FirmwareSelectError),
    #[error("title registry: {0}")]
    Registry(#[from] cellgov_boot::manifest::ManifestError),
    #[error("firmware {version}: {source}")]
    Manifest {
        version: String,
        #[source]
        source: FirmwareVerifyError,
    },
    #[error("firmware {version}: record PUP hash {record} disagrees with manifest {manifest}")]
    PupIdentity {
        version: String,
        record: String,
        manifest: String,
    },
    #[error("firmware {version}: {source}")]
    ModuleOpen {
        version: String,
        #[source]
        source: FirmwareVerifyError,
    },
    #[error("firmware {version} module {fault}")]
    ModuleDiverged {
        version: String,
        fault: Box<ModuleFault>,
    },
    #[error("firmware {version} module {module}: plaintext SHA-256 {found} disagrees with manifest {expected}")]
    ModuleModified {
        version: String,
        module: String,
        expected: String,
        found: String,
    },
    #[error("scan firmware {version} module {module}: {source}")]
    ModuleScan {
        version: String,
        module: String,
        #[source]
        source: CallerScanError,
    },
    #[error("map firmware {version} module {module}: {source}")]
    FunctionMap {
        version: String,
        module: String,
        #[source]
        source: FuncMapError,
    },
    #[error("compiled firmware.tsv: {0}")]
    FirmwareTable(#[source] ArchiveError),
    #[error("compiled firmware.tsv: {0}")]
    FirmwareRows(#[source] cellgov_lv2::archive::FirmwareTableError),
    #[error(transparent)]
    PupTable(#[from] crate::lv2_tables::CommittedPupError),
    #[error("read existing {}: {source}", path.display())]
    ExistingRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("existing caller census is partial under {}; all three tables must be present", path.display())]
    ExistingPartial { path: PathBuf },
    #[error("parse existing {table}.tsv: {source}")]
    ExistingParse {
        table: &'static str,
        #[source]
        source: ArchiveError,
    },
    #[error("render {table}.tsv: {source}")]
    Render {
        table: &'static str,
        #[source]
        source: ArchiveError,
    },
    #[error("create output directory {}: {source}", path.display())]
    CreateOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("output table path {} has no parent", path.display())]
    OutputParent { path: PathBuf },
    #[error("write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("key vault: {0}")]
    Vault(#[from] KeyVaultError),
}

pub(crate) fn run(
    args: &CallerCensusArgs,
    vfs_flag: Option<&Path>,
) -> Result<CommandExitCode, CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let mut tables = build(args, &vfs_root)
        .map_err(|error| CommandError::failed(format!("caller-census: {error}")))?;
    merge_existing(&args.output_dir, &mut tables)
        .map_err(|error| CommandError::failed(format!("caller-census: {error}")))?;
    write_tables(&args.output_dir, &tables)
        .map_err(|error| CommandError::failed(format!("caller-census: {error}")))?;
    println!(
        "caller-census: {} module(s), {} resolved site(s), {} unresolved site(s) -> {}",
        tables.modules,
        tables.resolved_sites,
        tables.unresolved_sites,
        args.output_dir.display()
    );
    Ok(CommandExitCode::SUCCESS)
}

fn build(args: &CallerCensusArgs, vfs_root: &Path) -> Result<CensusTables, CallerCensusError> {
    let store = crate::cli::keys::install_root_of(vfs_root);
    let inventory = StoreInventory::read(&store)?;
    let entries = selected_entries(args, &inventory)?;
    let vault = KeyVault::load_for_vfs(&store)?;
    let mut census = CallerCensus::default();
    let mut modules = 0usize;
    let mut resolved_sites = 0usize;
    let mut unresolved_sites = 0usize;

    for entry in entries {
        eprintln!("caller-census: firmware {}", entry.version);
        let manifest =
            firmware_verify::load_manifest(&entry.dev_flash_dir()).map_err(|source| {
                CallerCensusError::Manifest {
                    version: entry.version.clone(),
                    source,
                }
            })?;
        let pup_sha256 = manifest.firmware.pup_sha256.to_hex();
        if pup_sha256 != entry.pup_sha256 {
            return Err(CallerCensusError::PupIdentity {
                version: entry.version,
                record: entry.pup_sha256,
                manifest: pup_sha256,
            });
        }
        let mut files = manifest.files;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let dev_flash = entry.dev_flash_dir();
        for module in firmware_verify::module_images(&dev_flash, files, &vault) {
            let module = module.map_err(|source| CallerCensusError::ModuleOpen {
                version: entry.version.clone(),
                source,
            })?;
            let (module, elf) = matching_image(&entry.version, module)?;
            let Some(callers) = module_callers(&elf)
                .map_err(|source| analysis_refusal(&entry.version, &module, source))?
            else {
                continue;
            };
            modules += 1;
            resolved_sites += callers.resolved_sites();
            unresolved_sites += callers.unresolved.len();
            push_rows(&mut census, &pup_sha256, &module, callers);
        }
    }
    Ok(CensusTables {
        census,
        modules,
        resolved_sites,
        unresolved_sites,
    })
}

fn selected_entries(
    args: &CallerCensusArgs,
    inventory: &StoreInventory,
) -> Result<Vec<FirmwareEntry>, CallerCensusError> {
    if let Some(fw) = &args.fw {
        let managed = select_firmware(inventory, Some(fw), None)?;
        return Ok(vec![managed.entry]);
    }
    let registry = TitleRegistry::scan_dir(&crate::cli::store::registry_dir())?;
    let title_versions: BTreeSet<&str> = registry
        .iter()
        .flat_map(|manifest| manifest.matrix.iter().map(|cell| cell.key.fw.as_str()))
        .collect();
    let table =
        archive::parse(&FIRMWARE, FIRMWARE_TSV).map_err(CallerCensusError::FirmwareTable)?;
    let firmware_rows = archive::firmware_rows(&table);
    archive::check_firmware_rows(&firmware_rows).map_err(CallerCensusError::FirmwareRows)?;
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    for title_first in [true, false] {
        for row in &firmware_rows {
            if title_versions.contains(row.fw.as_str()) != title_first {
                continue;
            }
            if let Some(entry) = inventory.firmware(&row.fw) {
                seen.insert(row.fw.as_str());
                ordered.push(entry.clone());
            }
        }
    }
    for version in inventory.firmware_versions() {
        if !seen.contains(version.as_str()) {
            if let Some(entry) = inventory.firmware(&version) {
                ordered.push(entry.clone());
            }
        }
    }
    Ok(ordered)
}

/// A walked module's entry path and image, or the refusal for a module
/// that does not match its manifest entry.
fn matching_image(
    version: &str,
    module: ModuleImage,
) -> Result<(String, Vec<u8>), CallerCensusError> {
    match module.image {
        Ok(elf) => Ok((module.entry, elf)),
        Err(ModuleDivergence::Modified { expected, found }) => {
            Err(CallerCensusError::ModuleModified {
                version: version.to_string(),
                module: module.entry,
                expected: expected.to_hex(),
                found: found.to_hex(),
            })
        }
        Err(kind) => Err(CallerCensusError::ModuleDiverged {
            version: version.to_string(),
            fault: Box::new(ModuleFault {
                path: module.path,
                kind,
            }),
        }),
    }
}

fn analysis_refusal(version: &str, module: &str, error: ModuleCallersError) -> CallerCensusError {
    let (version, module) = (version.to_string(), module.to_string());
    match error {
        ModuleCallersError::Scan(source) => CallerCensusError::ModuleScan {
            version,
            module,
            source,
        },
        ModuleCallersError::FunctionMap(source) => CallerCensusError::FunctionMap {
            version,
            module,
            source,
        },
    }
}

/// One module's callers as the archive's rows: a caller row per
/// ordinal, one unresolved row, and a reach row per export and ordinal.
fn push_rows(census: &mut CallerCensus, pup_sha256: &str, module: &str, callers: ModuleCallers) {
    for (ordinal, sites) in callers.by_ordinal {
        census.caller.push(CallerRow {
            pup_sha256: pup_sha256.to_string(),
            module: module.to_string(),
            ordinal: usize::from(ordinal),
            sites,
        });
    }
    census.unresolved.push(CallerUnresolvedRow {
        pup_sha256: pup_sha256.to_string(),
        module: module.to_string(),
        sites: callers.unresolved,
    });
    for (nid, ordinal) in callers.reach {
        census.reach.push(ReachRow {
            pup_sha256: pup_sha256.to_string(),
            module: module.to_string(),
            export_nid: u64::from(nid),
            ordinal: usize::from(ordinal),
        });
    }
}

fn merge_existing(output_dir: &Path, tables: &mut CensusTables) -> Result<(), CallerCensusError> {
    let specs = [&CALLER, &CALLER_UNRESOLVED, &REACH];
    let paths: Vec<PathBuf> = specs
        .iter()
        .map(|spec| output_dir.join(spec.file()))
        .collect();
    let present = paths.iter().filter(|path| path.is_file()).count();
    if present == 0 {
        return Ok(());
    }
    if present != paths.len() {
        return Err(CallerCensusError::ExistingPartial {
            path: output_dir.to_path_buf(),
        });
    }
    let pups = crate::lv2_tables::committed_pup_rows()?;
    let valid: BTreeSet<&str> = pups.iter().map(|row| row.pup_sha256.as_str()).collect();
    let read = |spec: &'static archive::TableSpec| {
        let path = output_dir.join(spec.file());
        let text = std::fs::read_to_string(&path)
            .map_err(|source| CallerCensusError::ExistingRead { path, source })?;
        archive::parse(spec, &text).map_err(|source| CallerCensusError::ExistingParse {
            table: spec.name,
            source,
        })
    };
    let existing = CallerCensus {
        caller: archive::caller_rows(&read(&CALLER)?),
        unresolved: archive::caller_unresolved_rows(&read(&CALLER_UNRESOLVED)?),
        reach: archive::reach_rows(&read(&REACH)?),
    };
    tables.census.merge_existing(existing, &valid);
    Ok(())
}

fn write_tables(output_dir: &Path, tables: &CensusTables) -> Result<(), CallerCensusError> {
    std::fs::create_dir_all(output_dir).map_err(|source| CallerCensusError::CreateOutput {
        path: output_dir.to_path_buf(),
        source,
    })?;
    let census = &tables.census;
    for (spec, rendered) in [
        (&CALLER, archive::caller_tsv(&census.caller)),
        (
            &CALLER_UNRESOLVED,
            archive::caller_unresolved_tsv(&census.unresolved),
        ),
        (&REACH, archive::reach_tsv(&census.reach)),
    ] {
        let text = rendered.map_err(|source| CallerCensusError::Render {
            table: spec.name,
            source,
        })?;
        let path = output_dir.join(spec.file());
        let parent = path
            .parent()
            .ok_or_else(|| CallerCensusError::OutputParent { path: path.clone() })?;
        std::fs::create_dir_all(parent).map_err(|source| CallerCensusError::CreateOutput {
            path: parent.to_path_buf(),
            source,
        })?;
        std::fs::write(&path, text).map_err(|source| CallerCensusError::Write { path, source })?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/caller_census_tests.rs"]
mod tests;
