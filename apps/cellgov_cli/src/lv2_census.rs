//! Emits one LV2 kernel's deterministic archive rows.
//!
//! The command maps `cellgov_ppu`'s kernel classification into the
//! archive's row types; `cellgov_lv2::archive` holds the rows and the
//! rules that fold them into the archive.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_lv2::archive::{
    self, CensusClass, CensusRow, DispatchShape, ExtractedRows, ExtractionError, GateRow,
    GateState, KernelRow, PupExtraction, StubRow, SubentryRow, CAPABILITY_GATE, CENSUS, KERNEL,
    STUB, SUBENTRY,
};
use cellgov_ppu::lv2_gate::{self, Lv2Gate, Lv2GateRead};
use cellgov_ppu::lv2_stub::Lv2OrdinalClass;
use cellgov_ppu::lv2_subdispatch::{self, Lv2Subdispatch, Lv2SubdispatchError, Lv2SubentryClass};

use crate::cli::exit::CommandError;
use crate::cli::parse::Lv2CensusArgs;
use crate::cli::self_load::{decrypt_ppu_self, load_file};

#[derive(Debug, thiserror::Error)]
enum Lv2CensusError {
    #[error("classify kernel: {0}")]
    Classification(#[from] Lv2SubdispatchError),
    #[error(transparent)]
    PupTable(#[from] crate::lv2_tables::CommittedPupError),
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
    #[error("{0}; pass --replace-version to accept the movement")]
    Movement(#[source] ExtractionError),
    #[error("existing kernel census is partial under {}; kernel.tsv, stub.tsv, subentry.tsv, and gate.tsv must all be present", path.display())]
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
    #[error("firmware {fw} census differs from the existing {}", path.display())]
    CensusConflict { fw: String, path: PathBuf },
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

pub(crate) fn run(args: &Lv2CensusArgs, vfs_flag: Option<&Path>) -> Result<(), CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let raw = load_file(&args.path.to_string_lossy())?;
    let elf = decrypt_ppu_self(&raw, &args.path.to_string_lossy(), &vfs_root)?;
    let summary =
        emit(args, &elf).map_err(|error| CommandError::failed(format!("lv2-census: {error}")))?;
    println!(
        "lv2-census: firmware {} PUP {} -> {} ordinals, {} stub targets, {} subentries, {} gated ordinals, {} other same-version PUP rows removed under {}",
        args.fw,
        args.pup_sha256,
        summary.ordinals,
        summary.stub_targets,
        summary.subentries,
        summary.gated,
        summary.removed_pups,
        args.output_dir.display()
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmitSummary {
    ordinals: usize,
    stub_targets: usize,
    subentries: usize,
    gated: usize,
    removed_pups: usize,
}

fn emit(args: &Lv2CensusArgs, elf: &[u8]) -> Result<EmitSummary, Lv2CensusError> {
    let pups = crate::lv2_tables::committed_pup_rows()?;
    archive::select_pup(&pups, &args.pup_sha256, &args.fw)?;

    let subdispatch = lv2_subdispatch::classify(elf)?;
    let classification = &subdispatch.top_level;
    let census_rows: Vec<CensusRow> = classification
        .ordinals
        .iter()
        .map(|entry| CensusRow {
            fw: args.fw.clone(),
            ordinal: entry.ordinal,
            class: match entry.class {
                Lv2OrdinalClass::Implemented => CensusClass::Implemented,
                Lv2OrdinalClass::Stub => CensusClass::Stub,
                Lv2OrdinalClass::Absent => CensusClass::Absent,
            },
            target: entry.code,
            dispatch: match subdispatch.ordinals.get(&entry.ordinal) {
                Some(Lv2Subdispatch::Table { .. }) => DispatchShape::Subtable,
                Some(Lv2Subdispatch::ChainIncomplete { .. }) => DispatchShape::ChainIncomplete,
                None => DispatchShape::Flat,
            },
        })
        .collect();
    let census_text =
        archive::census_tsv(&census_rows).map_err(|source| Lv2CensusError::Render {
            table: CENSUS.name,
            source,
        })?;
    let census_sha256 = sha256_hex(census_text.as_bytes());

    let new_subentries = build_subentry_rows(&args.pup_sha256, &subdispatch.ordinals);
    let subentry_sha256 = archive::subentry_digest(&new_subentries, &sha256_hex)?;
    let new_subentry_count = new_subentries.len();
    let gate_classification =
        lv2_gate::classify(elf, classification).map_err(Lv2SubdispatchError::from)?;
    let new_gates = build_gate_rows(&args.pup_sha256, &gate_classification);
    let gate_sha256 = archive::gate_digest(&new_gates, &sha256_hex)?;
    let gated_count = new_gates
        .iter()
        .filter(|row| row.state == GateState::Gated)
        .count();
    let mut existing = load_existing(&args.output_dir)?;
    archive::validate_existing(&existing, &pups, &sha256_hex)?;
    let new_kernel = KernelRow {
        pup_sha256: args.pup_sha256.clone(),
        kernel_elf_sha256: sha256_hex(elf),
        table_base: classification.discovery.table_vaddr,
        entry_width: classification.discovery.entry_width,
        entry_format: classification.discovery.entry_format.as_str().to_string(),
        entry_count: classification.discovery.entry_count,
        discovery_method: classification.discovery.method.as_str().to_string(),
        confidence: classification.discovery.confidence.as_str().to_string(),
        census_sha256,
        subentry_sha256,
        gate_sha256,
    };
    let new_stubs: Vec<StubRow> = classification
        .stub_targets
        .iter()
        .map(|stub| StubRow {
            pup_sha256: args.pup_sha256.clone(),
            descriptor: stub.descriptor,
            target: stub.code,
            errno: stub.errno,
            errno_symbol: stub.errno_symbol.to_string(),
            references: stub.references,
            primary: stub.descriptor == classification.primary_stub.descriptor,
        })
        .collect();
    let removed_pups = archive::merge_extraction(
        &mut existing,
        PupExtraction {
            kernel: new_kernel,
            stubs: new_stubs,
            subentries: new_subentries,
            gates: new_gates,
        },
        &args.fw,
        &pups,
        args.replace_version,
    )
    .map_err(merge_refusal)?;
    let kernel_text =
        archive::kernel_tsv(&existing.kernels).map_err(|source| Lv2CensusError::Render {
            table: KERNEL.name,
            source,
        })?;
    let stub_text =
        archive::stub_tsv(&existing.stubs).map_err(|source| Lv2CensusError::Render {
            table: STUB.name,
            source,
        })?;
    let subentry_text =
        archive::subentry_tsv(&existing.subentries).map_err(|source| Lv2CensusError::Render {
            table: SUBENTRY.name,
            source,
        })?;
    let gate_text =
        archive::gate_tsv(&existing.gates).map_err(|source| Lv2CensusError::Render {
            table: CAPABILITY_GATE.name,
            source,
        })?;
    write_all(
        args,
        &census_text,
        &kernel_text,
        &stub_text,
        &subentry_text,
        &gate_text,
    )?;
    Ok(EmitSummary {
        ordinals: census_rows.len(),
        stub_targets: classification.stub_targets.len(),
        subentries: new_subentry_count,
        gated: gated_count,
        removed_pups,
    })
}

/// A merge refusal as the command reports it: a moved re-extraction
/// names the flag that accepts it.
fn merge_refusal(error: ExtractionError) -> Lv2CensusError {
    match error {
        ExtractionError::ExtractionConflict { .. } => Lv2CensusError::Movement(error),
        other => other.into(),
    }
}

fn build_subentry_rows(
    pup_sha256: &str,
    dispatches: &BTreeMap<usize, Lv2Subdispatch>,
) -> Vec<SubentryRow> {
    let mut rows = Vec::new();
    for (ordinal, dispatch) in dispatches {
        let Lv2Subdispatch::Table {
            selector_slot,
            entries,
        } = dispatch
        else {
            continue;
        };
        let selector_slot = archive::selector_slot_name(*selector_slot);
        rows.extend(entries.iter().map(|entry| SubentryRow {
            pup_sha256: pup_sha256.to_string(),
            ordinal: *ordinal,
            selector_slot: selector_slot.clone(),
            packet: entry.packet,
            class: match entry.class {
                Lv2SubentryClass::Implemented => CensusClass::Implemented,
                Lv2SubentryClass::Stub => CensusClass::Stub,
            },
            target: entry.target,
        }));
    }
    rows
}

fn build_gate_rows(pup_sha256: &str, gates: &BTreeMap<usize, Lv2Gate>) -> Vec<GateRow> {
    gates
        .iter()
        .map(|(ordinal, gate)| {
            let (state, reads, fail_errno) = match gate {
                Lv2Gate::Gated { reads, fail_errno } => (
                    GateState::Gated,
                    Some(match reads {
                        Lv2GateRead::ControlFlags1(mask) => archive::control_flags1_read(*mask),
                    }),
                    Some(*fail_errno),
                ),
                Lv2Gate::Ungated => (GateState::Ungated, None, None),
                Lv2Gate::NotAnalysed => (GateState::NotAnalysed, None, None),
            };
            GateRow {
                pup_sha256: pup_sha256.to_string(),
                ordinal: *ordinal,
                state,
                reads,
                fail_errno,
            }
        })
        .collect()
}

fn load_existing(output_dir: &Path) -> Result<ExtractedRows, Lv2CensusError> {
    let kernel_path = output_dir.join(KERNEL.file());
    let stub_path = output_dir.join(STUB.file());
    let subentry_path = output_dir.join(SUBENTRY.file());
    let gate_path = output_dir.join(CAPABILITY_GATE.file());
    match (
        kernel_path.is_file(),
        stub_path.is_file(),
        subentry_path.is_file(),
        gate_path.is_file(),
    ) {
        (false, false, false, false) => Ok(ExtractedRows::default()),
        (true, true, true, true) => {
            let kernel = read_table(&kernel_path, &KERNEL)?;
            let stub = read_table(&stub_path, &STUB)?;
            let subentry = archive::subentry_rows(&read_table(&subentry_path, &SUBENTRY)?);
            let gate = archive::gate_rows(&read_table(&gate_path, &CAPABILITY_GATE)?);
            Ok(ExtractedRows {
                kernels: archive::kernel_rows(&kernel),
                stubs: archive::stub_rows(&stub),
                subentries: subentry,
                gates: gate,
            })
        }
        _ => Err(Lv2CensusError::ExistingPartial {
            path: output_dir.to_path_buf(),
        }),
    }
}

fn read_table(
    path: &Path,
    spec: &'static archive::TableSpec,
) -> Result<archive::Table, Lv2CensusError> {
    let text = std::fs::read_to_string(path).map_err(|source| Lv2CensusError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    archive::parse(spec, &text).map_err(|source| Lv2CensusError::Parse {
        table: spec.name,
        source,
    })
}

fn write_all(
    args: &Lv2CensusArgs,
    census_text: &str,
    kernel_text: &str,
    stub_text: &str,
    subentry_text: &str,
    gate_text: &str,
) -> Result<(), Lv2CensusError> {
    let census_dir = args.output_dir.join("census");
    std::fs::create_dir_all(&census_dir).map_err(|source| Lv2CensusError::CreateOutput {
        path: census_dir.clone(),
        source,
    })?;
    let census_path = args.output_dir.join(archive::census_file(&args.fw));
    let existing = if census_path.is_file() {
        Some(
            std::fs::read_to_string(&census_path).map_err(|source| Lv2CensusError::Read {
                path: census_path.clone(),
                source,
            })?,
        )
    } else {
        None
    };
    let write_census = match archive::census_needs_write(
        existing.as_deref(),
        census_text,
        &args.fw,
        args.replace_version,
    ) {
        Ok(write_census) => write_census,
        Err(ExtractionError::CensusConflict { fw }) => {
            return Err(Lv2CensusError::CensusConflict {
                fw,
                path: census_path,
            })
        }
        Err(error) => return Err(error.into()),
    };
    if write_census {
        write(&census_path, census_text)?;
    }
    write(&args.output_dir.join(KERNEL.file()), kernel_text)?;
    write(&args.output_dir.join(STUB.file()), stub_text)?;
    write(&args.output_dir.join(SUBENTRY.file()), subentry_text)?;
    write(&args.output_dir.join(CAPABILITY_GATE.file()), gate_text)?;
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<(), Lv2CensusError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| Lv2CensusError::CreateOutput {
        path: parent.to_path_buf(),
        source,
    })?;
    std::fs::write(path, text).map_err(|source| Lv2CensusError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256(sha256_of(bytes)).to_hex()
}

#[cfg(test)]
#[path = "tests/lv2_census_tests.rs"]
mod tests;
