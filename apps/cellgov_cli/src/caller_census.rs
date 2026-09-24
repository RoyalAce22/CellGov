//! Emits firmware PPU syscall caller tables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::TitleRegistry;
use cellgov_install::firmware_verify::{self, FirmwareVerifyError};
use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_install::sce::{self, SceError};
use cellgov_lv2::archive::{self, ArchiveError, CALLER, CALLER_UNRESOLVED, FIRMWARE, PUP, REACH};
use cellgov_ppu::caller_census::{scan_syscalls, CallerScanError};
use cellgov_ppu::funcmap::{self, FuncMapError, FunctionName};
use cellgov_ps3_abi::format::elf::{ELF_E_MACHINE_OFFSET, ELF_HEADER_SIZE, ELF_MAGIC, EM_PPC64};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::CallerCensusArgs;
use crate::composition::refusal::firmware_refusal;
use cellgov_install::store::inventory::{FirmwareEntry, InventoryError, StoreInventory};
use cellgov_install::store::select::{select_firmware, FirmwareSelectError};

const FIRMWARE_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/firmware.tsv"
));

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/pup.tsv"
));

struct CensusTables {
    caller: Vec<Vec<String>>,
    unresolved: Vec<Vec<String>>,
    reach: Vec<Vec<String>>,
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
    #[error("read firmware {version} module {}: {source}", path.display())]
    ModuleRead {
        version: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("decrypt firmware {version} module {module}: {source}")]
    ModuleDecrypt {
        version: String,
        module: String,
        #[source]
        source: SceError,
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
    #[error("compiled pup.tsv: {0}")]
    PupTable(#[source] ArchiveError),
    #[error("compiled pup.tsv: {0}")]
    PupRows(#[source] cellgov_lv2::archive::PupTableError),
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
    let mut caller = Vec::new();
    let mut unresolved = Vec::new();
    let mut reach = Vec::new();
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
        for file in files {
            let path = file
                .path
                .split('/')
                .fold(entry.dev_flash_dir(), |path, part| path.join(part));
            let raw = std::fs::read(&path).map_err(|source| CallerCensusError::ModuleRead {
                version: entry.version.clone(),
                path: path.clone(),
                source,
            })?;
            let elf = if raw.starts_with(&ELF_MAGIC) {
                raw
            } else {
                sce::decrypt_self_to_elf(&raw, &vault).map_err(|source| {
                    CallerCensusError::ModuleDecrypt {
                        version: entry.version.clone(),
                        module: file.path.clone(),
                        source,
                    }
                })?
            };
            verify_module_hash(&entry.version, &file.path, file.sha256, &elf)?;
            if !is_ppu_elf(&elf) {
                continue;
            }
            let sites = scan_syscalls(&elf).map_err(|source| CallerCensusError::ModuleScan {
                version: entry.version.clone(),
                module: file.path.clone(),
                source,
            })?;
            let functions =
                funcmap::build(&elf).map_err(|source| CallerCensusError::FunctionMap {
                    version: entry.version.clone(),
                    module: file.path.clone(),
                    source,
                })?;
            modules += 1;
            let mut by_ordinal: BTreeMap<u16, Vec<u64>> = BTreeMap::new();
            let mut unknown = Vec::new();
            let mut module_reach = BTreeSet::new();
            for site in sites {
                match site.ordinal {
                    Some(ordinal) => {
                        resolved_sites += 1;
                        by_ordinal.entry(ordinal).or_default().push(site.address);
                        if let Ok(address) = u32::try_from(site.address) {
                            if let Some(span) = functions.span_at(address) {
                                if let FunctionName::Nid(nid) = span.name {
                                    module_reach.insert((nid, ordinal));
                                }
                            }
                        }
                    }
                    None => {
                        unresolved_sites += 1;
                        unknown.push(site.address);
                    }
                }
            }
            for (ordinal, sites) in by_ordinal {
                caller.push(vec![
                    pup_sha256.clone(),
                    file.path.clone(),
                    ordinal.to_string(),
                    integer_list(&sites),
                ]);
            }
            unresolved.push(vec![
                pup_sha256.clone(),
                file.path.clone(),
                if unknown.is_empty() {
                    archive::NONE.to_string()
                } else {
                    integer_list(&unknown)
                },
            ]);
            for (nid, ordinal) in module_reach {
                reach.push(vec![
                    pup_sha256.clone(),
                    file.path.clone(),
                    nid.to_string(),
                    ordinal.to_string(),
                ]);
            }
        }
    }
    let mut tables = CensusTables {
        caller,
        unresolved,
        reach,
        modules,
        resolved_sites,
        unresolved_sites,
    };
    canonicalize(&mut tables);
    Ok(tables)
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

fn is_ppu_elf(elf: &[u8]) -> bool {
    elf.len() >= ELF_HEADER_SIZE
        && u16::from_be_bytes([elf[ELF_E_MACHINE_OFFSET], elf[ELF_E_MACHINE_OFFSET + 1]])
            == EM_PPC64
}

fn verify_module_hash(
    version: &str,
    module: &str,
    expected: Sha256,
    elf: &[u8],
) -> Result<(), CallerCensusError> {
    let found = Sha256(sha256_of(elf));
    if found == expected {
        return Ok(());
    }
    Err(CallerCensusError::ModuleModified {
        version: version.to_string(),
        module: module.to_string(),
        expected: expected.to_hex(),
        found: found.to_hex(),
    })
}

fn integer_list(values: &[u64]) -> String {
    values
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn sort_rows(rows: &mut [Vec<String>], text: &[usize], integers: &[usize]) {
    rows.sort_by(|a, b| {
        for index in text {
            let order = a[*index].cmp(&b[*index]);
            if !order.is_eq() {
                return order;
            }
        }
        for index in integers {
            let order = a[*index]
                .len()
                .cmp(&b[*index].len())
                .then_with(|| a[*index].cmp(&b[*index]));
            if !order.is_eq() {
                return order;
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn canonicalize(tables: &mut CensusTables) {
    sort_rows(&mut tables.caller, &[0, 1], &[2]);
    sort_rows(&mut tables.unresolved, &[0, 1], &[]);
    sort_rows(&mut tables.reach, &[0, 1], &[2, 3]);
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
    let pup_table = archive::parse(&PUP, PUP_TSV).map_err(CallerCensusError::PupTable)?;
    let pups = archive::pup_rows(&pup_table);
    archive::check_pup_rows(&pups).map_err(CallerCensusError::PupRows)?;
    let valid: BTreeSet<String> = pups.into_iter().map(|row| row.pup_sha256).collect();
    let selected: BTreeSet<String> = tables.unresolved.iter().map(|row| row[0].clone()).collect();
    for ((spec, path), target) in specs.into_iter().zip(paths).zip([
        &mut tables.caller,
        &mut tables.unresolved,
        &mut tables.reach,
    ]) {
        let text = std::fs::read_to_string(&path)
            .map_err(|source| CallerCensusError::ExistingRead { path, source })?;
        let existing =
            archive::parse(spec, &text).map_err(|source| CallerCensusError::ExistingParse {
                table: spec.name,
                source,
            })?;
        target.extend(
            existing
                .rows
                .into_iter()
                .filter(|row| valid.contains(&row[0]) && !selected.contains(&row[0])),
        );
    }
    canonicalize(tables);
    Ok(())
}

fn write_tables(output_dir: &Path, tables: &CensusTables) -> Result<(), CallerCensusError> {
    std::fs::create_dir_all(output_dir).map_err(|source| CallerCensusError::CreateOutput {
        path: output_dir.to_path_buf(),
        source,
    })?;
    for (spec, rows) in [
        (&CALLER, &tables.caller),
        (&CALLER_UNRESOLVED, &tables.unresolved),
        (&REACH, &tables.reach),
    ] {
        let text = archive::render(spec, rows).map_err(|source| CallerCensusError::Render {
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
