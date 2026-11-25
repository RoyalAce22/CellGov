//! Emits one LV2 kernel's LV1 hypercall sites into the archive's two
//! hypercall site tables.
//!
//! `cellgov_ppu` finds the sites; `cellgov_lv2_archive` holds the rows
//! and renders the tables. The command replaces the rows of the named
//! PUP and keeps every other PUP's rows. It scans only the kernel ELF
//! that `kernel.tsv` names for that PUP.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_lv2_archive::{
    self as archive, ExtractionError, HypercallCensus, HypercallSiteRow, HypercallUnresolvedRow,
    HYPERCALL_SITE, HYPERCALL_UNRESOLVED, KERNEL,
};
use cellgov_ppu::caller_census::{scan_hypercalls, CallerScanError, CallerSite};
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;

use crate::cli::exit::CommandError;
use crate::cli::parse::HypercallCensusArgs;
use crate::cli::self_load::{decrypt_ppu_self, load_file};

/// The committed `kernel.tsv`, which holds the SHA-256 of each PUP's
/// decrypted kernel ELF.
const KERNEL_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/kernel.tsv"
));

#[derive(Debug, thiserror::Error)]
enum HypercallCensusError {
    #[error(transparent)]
    PupTable(#[from] crate::lv2_tables::CommittedPupError),
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
    #[error("parse compiled kernel.tsv: {source}")]
    KernelTable {
        #[source]
        source: archive::ArchiveError,
    },
    #[error("PUP {pup_sha256} has no kernel.tsv row; run lv2-census for it first")]
    NoKernelRow { pup_sha256: String },
    #[error(
        "kernel ELF SHA-256 {found} is not the kernel {expected} that kernel.tsv names for PUP \
         {pup_sha256}"
    )]
    KernelMismatch {
        pup_sha256: String,
        expected: String,
        found: String,
    },
    #[error("scan kernel: {0}")]
    Scan(#[from] CallerScanError),
    #[error(
        "existing hypercall census is partial under {}; hypercall_site.tsv and \
         hypercall_unresolved.tsv must both be present or both absent",
        path.display()
    )]
    ExistingPartial { path: PathBuf },
    #[error("read existing {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse existing {table}: {source}")]
    Parse {
        table: &'static str,
        #[source]
        source: archive::ArchiveError,
    },
    #[error("render {table}: {source}")]
    Render {
        table: &'static str,
        #[source]
        source: archive::ArchiveError,
    },
    #[error("create output directory {}: {source}", path.display())]
    CreateOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn run(args: &HypercallCensusArgs, vfs_flag: Option<&Path>) -> Result<(), CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let raw = load_file(&args.path.to_string_lossy())?;
    let elf = decrypt_ppu_self(&raw, &args.path.to_string_lossy(), &vfs_root)?;
    let summary = emit(args, &elf)
        .map_err(|error| CommandError::failed(format!("hypercall-census: {error}")))?;
    println!(
        "hypercall-census: firmware {} PUP {} -> {} numbers, {} resolved sites, {} unresolved sites under {}",
        args.fw,
        args.pup_sha256,
        summary.numbers,
        summary.resolved,
        summary.unresolved,
        args.output_dir.display()
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmitSummary {
    numbers: usize,
    resolved: usize,
    unresolved: usize,
}

fn emit(args: &HypercallCensusArgs, elf: &[u8]) -> Result<EmitSummary, HypercallCensusError> {
    let pups = crate::lv2_tables::committed_pup_rows()?;
    archive::select_pup(&pups, &args.pup_sha256, &args.fw)?;
    check_kernel(&args.pup_sha256, elf)?;
    write_census(args, elf)
}

/// Refuses `elf` unless `kernel.tsv` names it as the kernel of
/// `pup_sha256`.
///
/// This keeps a PUP's rows to the sites of its own kernel.
fn check_kernel(pup_sha256: &str, elf: &[u8]) -> Result<(), HypercallCensusError> {
    let table = archive::parse(&KERNEL, KERNEL_TSV)
        .map_err(|source| HypercallCensusError::KernelTable { source })?;
    let expected = archive::kernel_rows(&table)
        .into_iter()
        .find(|row| row.pup_sha256 == pup_sha256)
        .ok_or_else(|| HypercallCensusError::NoKernelRow {
            pup_sha256: pup_sha256.to_string(),
        })?
        .kernel_elf_sha256;
    let found = Sha256(sha256_of(elf)).to_hex();
    if found != expected {
        return Err(HypercallCensusError::KernelMismatch {
            pup_sha256: pup_sha256.to_string(),
            expected,
            found,
        });
    }
    Ok(())
}

/// Scans `elf` and writes its rows over the PUP's rows in the output
/// directory.
fn write_census(
    args: &HypercallCensusArgs,
    elf: &[u8],
) -> Result<EmitSummary, HypercallCensusError> {
    let scan = census_of(&args.pup_sha256, &scan_hypercalls(elf)?);
    let summary = EmitSummary {
        numbers: scan.sites.len(),
        resolved: scan.sites.iter().map(|row| row.sites.len()).sum(),
        unresolved: scan.unresolved.iter().map(|row| row.sites.len()).sum(),
    };
    let mut census = load_existing(&args.output_dir)?;
    census.replace_pup(&args.pup_sha256, scan);
    let site_text = render(&HYPERCALL_SITE, archive::hypercall_site_tsv(&census.sites))?;
    let unresolved_text = render(
        &HYPERCALL_UNRESOLVED,
        archive::hypercall_unresolved_tsv(&census.unresolved),
    )?;
    write(&args.output_dir.join(HYPERCALL_SITE.file()), &site_text)?;
    write(
        &args.output_dir.join(HYPERCALL_UNRESOLVED.file()),
        &unresolved_text,
    )?;
    Ok(summary)
}

/// The rows of one kernel's sites: each number's real addresses, and the
/// unresolved ones.
fn census_of(pup_sha256: &str, sites: &[CallerSite]) -> HypercallCensus {
    let mut by_number: BTreeMap<u16, Vec<u64>> = BTreeMap::new();
    let mut unresolved = Vec::new();
    for site in sites {
        let real = site.address & !REAL_MODE_IGNORED_EA;
        match site.ordinal {
            Some(number) => by_number.entry(number).or_default().push(real),
            None => unresolved.push(real),
        }
    }
    HypercallCensus {
        sites: by_number
            .into_iter()
            .map(|(number, mut sites)| {
                sites.sort_unstable();
                HypercallSiteRow {
                    pup_sha256: pup_sha256.to_string(),
                    number: u64::from(number),
                    sites,
                }
            })
            .collect(),
        unresolved: vec![HypercallUnresolvedRow {
            pup_sha256: pup_sha256.to_string(),
            sites: {
                unresolved.sort_unstable();
                unresolved
            },
        }],
    }
}

/// One table's rendered text, or the render error naming the table.
fn render(
    table: &archive::TableSpec,
    rendered: Result<String, archive::ArchiveError>,
) -> Result<String, HypercallCensusError> {
    rendered.map_err(|source| HypercallCensusError::Render {
        table: table.name,
        source,
    })
}

fn load_existing(output_dir: &Path) -> Result<HypercallCensus, HypercallCensusError> {
    let site_path = output_dir.join(HYPERCALL_SITE.file());
    let unresolved_path = output_dir.join(HYPERCALL_UNRESOLVED.file());
    match (site_path.is_file(), unresolved_path.is_file()) {
        (false, false) => Ok(HypercallCensus::default()),
        (true, true) => Ok(HypercallCensus {
            sites: archive::hypercall_site_rows(&read_table(&site_path, &HYPERCALL_SITE)?),
            unresolved: archive::hypercall_unresolved_rows(&read_table(
                &unresolved_path,
                &HYPERCALL_UNRESOLVED,
            )?),
        }),
        _ => Err(HypercallCensusError::ExistingPartial {
            path: output_dir.to_path_buf(),
        }),
    }
}

fn read_table(
    path: &Path,
    spec: &'static archive::TableSpec,
) -> Result<archive::Table, HypercallCensusError> {
    let text = std::fs::read_to_string(path).map_err(|source| HypercallCensusError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    archive::parse(spec, &text).map_err(|source| HypercallCensusError::Parse {
        table: spec.name,
        source,
    })
}

fn write(path: &Path, text: &str) -> Result<(), HypercallCensusError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| HypercallCensusError::CreateOutput {
        path: parent.to_path_buf(),
        source,
    })?;
    std::fs::write(path, text).map_err(|source| HypercallCensusError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[path = "tests/hypercall_census_tests.rs"]
mod tests;
