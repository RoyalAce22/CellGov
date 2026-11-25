//! Boots one LV2 kernel from its entry under CellGov's hypervisor and
//! emits the repository reads it makes into the archive's
//! `repository_read.tsv`.
//!
//! `cellgov_boot` runs the kernel; `cellgov_lv1_archive` holds the rows
//! and renders the table. The command replaces the rows of the named PUP
//! and keeps every other PUP's rows. It boots only the kernel ELF that
//! `kernel.tsv` names for that PUP.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cellgov_boot::kernel::{run_kernel_from_entry, KernelRun, KernelRunError, KernelRunStop};
use cellgov_lv1::HypercallRefusal;
use cellgov_lv1_archive::{self as lv1_archive, RepositoryReadRow, REPOSITORY_READ};
use cellgov_lv2_archive::{self as archive, ExtractionError};

use crate::cli::exit::CommandError;
use crate::cli::parse::RepositoryCensusArgs;
use crate::cli::self_load::{decrypt_ppu_self, load_file};
use crate::lv2_tables::{check_committed_kernel, CommittedKernelError};

#[derive(Debug, thiserror::Error)]
enum RepositoryCensusError {
    #[error(transparent)]
    PupTable(#[from] crate::lv2_tables::CommittedPupError),
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
    #[error(transparent)]
    Kernel(#[from] CommittedKernelError),
    #[error("boot kernel: {0}")]
    Boot(#[from] KernelRunError),
    #[error(
        "the boot reached the step cap of {max_steps} after {steps} steps and {reads} repository \
         reads, before CellGov's hypervisor stopped it; raise --max-steps"
    )]
    StepCap {
        max_steps: usize,
        steps: usize,
        reads: usize,
    },
    #[error(transparent)]
    MixedRead(#[from] lv1_archive::MixedRepositoryRead),
    #[error("read existing {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse existing {}: {source}", REPOSITORY_READ.name)]
    Parse {
        #[source]
        source: archive::ArchiveError,
    },
    #[error("render {}: {source}", REPOSITORY_READ.name)]
    Render {
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

pub(crate) fn run(
    args: &RepositoryCensusArgs,
    vfs_flag: Option<&Path>,
) -> Result<(), CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let raw = load_file(&args.path.to_string_lossy())?;
    let elf = decrypt_ppu_self(&raw, &args.path.to_string_lossy(), &vfs_root)?;
    let boot = emit(args, &elf)
        .map_err(|error| CommandError::failed(format!("repository-census: {error}")))?;
    println!(
        "repository-census: firmware {} PUP {} -> {} reads of {} nodes in {} steps; stopped at {} \
         ({} under {})",
        args.fw,
        args.pup_sha256,
        boot.repository_reads.len(),
        boot.repository_reads
            .iter()
            .map(|read| read.key)
            .collect::<BTreeSet<_>>()
            .len(),
        boot.steps,
        stop_text(&boot.stop),
        REPOSITORY_READ.file(),
        args.output_dir.display()
    );
    Ok(())
}

/// One line that names where a kernel boot stopped.
fn stop_text(stop: &KernelRunStop) -> String {
    match stop {
        KernelRunStop::Refused(HypercallRefusal::RepositoryNode { key, pc, .. }) => {
            format!("a refused repository read of {key} at pc 0x{pc:016x}")
        }
        KernelRunStop::Refused(refusal) => format!("a refused hypercall: {refusal}"),
        KernelRunStop::Fault {
            fault,
            pc: Some(pc),
        } => {
            format!("a kernel fault {fault:?} at pc 0x{pc:016x}")
        }
        KernelRunStop::Fault { fault, pc: None } => format!("a kernel fault {fault:?}"),
        KernelRunStop::SiblingFault {
            fault,
            pc: Some(pc),
        } => {
            format!("a placeholder sibling fault {fault:?} at pc 0x{pc:016x}")
        }
        KernelRunStop::SiblingFault { fault, pc: None } => {
            format!("a placeholder sibling fault {fault:?}")
        }
        KernelRunStop::StepCap => "the step cap".to_string(),
        KernelRunStop::Step(error) => format!("a step refusal: {error}"),
        KernelRunStop::Commit(error) => format!("a commit refusal: {error}"),
    }
}

/// Boots the kernel `elf` of the PUP `args` names and writes its reads
/// over that PUP's rows in the output directory.
fn emit(args: &RepositoryCensusArgs, elf: &[u8]) -> Result<KernelRun, RepositoryCensusError> {
    let pups = crate::lv2_tables::committed_pup_rows()?;
    archive::select_pup(&pups, &args.pup_sha256, &args.fw)?;
    check_committed_kernel(&args.pup_sha256, elf)?;
    let boot = run_kernel_from_entry(elf, &args.fw, args.max_steps)?;
    let fresh = census_rows(&args.pup_sha256, args.max_steps, &boot)?;
    let path = args.output_dir.join(REPOSITORY_READ.file());
    let mut rows = load_existing(&path)?;
    lv1_archive::replace_repository_reads(&mut rows, &args.pup_sha256, fresh);
    let text = lv1_archive::repository_read_tsv(&rows)
        .map_err(|source| RepositoryCensusError::Render { source })?;
    write(&path, &text)?;
    Ok(boot)
}

/// The rows of `boot`, a run of the kernel of the PUP `pup_sha256` capped
/// at `max_steps`.
///
/// A run cut off by its step cap is refused, so its rows never replace a
/// logged boot.
// The step cap is a deadlock bound that callers abort on (cellgov_core
// `StepError::MaxStepsExceeded`). The archive logs each boot's reads up to
// where CellGov's hypervisor stops it (docs/lv1/README.md, "Hypervisor
// repository"); a capped run stops where the cap fell instead.
fn census_rows(
    pup_sha256: &str,
    max_steps: usize,
    boot: &KernelRun,
) -> Result<Vec<RepositoryReadRow>, RepositoryCensusError> {
    if matches!(boot.stop, KernelRunStop::StepCap) {
        return Err(RepositoryCensusError::StepCap {
            max_steps,
            steps: boot.steps,
            reads: boot.repository_reads.len(),
        });
    }
    lv1_archive::repository_read_rows_of(pup_sha256, &boot.repository_reads)
        .map_err(RepositoryCensusError::from)
}

/// The rows of the table at `path`, or none when no table is there.
fn load_existing(path: &Path) -> Result<Vec<RepositoryReadRow>, RepositoryCensusError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RepositoryCensusError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    archive::parse(&REPOSITORY_READ, &text)
        .and_then(|table| lv1_archive::repository_read_rows(&table))
        .map_err(|source| RepositoryCensusError::Parse { source })
}

fn write(path: &Path, text: &str) -> Result<(), RepositoryCensusError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| RepositoryCensusError::CreateOutput {
        path: parent.to_path_buf(),
        source,
    })?;
    std::fs::write(path, text).map_err(|source| RepositoryCensusError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[path = "tests/repository_census_tests.rs"]
mod tests;
