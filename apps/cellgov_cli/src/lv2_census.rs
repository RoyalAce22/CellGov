//! Emits one LV2 kernel's deterministic archive rows.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_lv2::archive::{
    self, CensusClass, CensusRow, DispatchShape, GateRow, GateState, KernelRow, PupRow, StubRow,
    SubentryRow, CAPABILITY_GATE, CENSUS, KERNEL, PUP, STUB, SUBENTRY,
};
use cellgov_ppu::lv2_gate::{self, Lv2Gate, Lv2GateRead};
use cellgov_ppu::lv2_stub::Lv2OrdinalClass;
use cellgov_ppu::lv2_subdispatch::{self, Lv2Subdispatch, Lv2SubdispatchError, Lv2SubentryClass};

use crate::cli::exit::CommandError;
use crate::cli::parse::Lv2CensusArgs;
use crate::cli::self_load::{decrypt_ppu_self, load_file};

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/pup.tsv"
));

#[derive(Debug, thiserror::Error)]
enum Lv2CensusError {
    #[error("classify kernel: {0}")]
    Classification(#[from] Lv2SubdispatchError),
    #[error("compiled pup.tsv: {0}")]
    PupTable(#[source] archive::ArchiveError),
    #[error("PUP {pup_sha256} is not recorded in pup.tsv")]
    UnknownPup { pup_sha256: String },
    #[error("PUP {pup_sha256} belongs to firmware {recorded}, not {requested}")]
    FirmwareMismatch {
        pup_sha256: String,
        recorded: String,
        requested: String,
    },
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
    #[error("firmware {fw} kernel rows disagree on their census digest")]
    DigestConflict { fw: String },
    #[error(
        "PUP {pup_sha256} re-extracted different kernel or stub rows; pass --replace-version to accept the movement"
    )]
    ExtractionConflict { pup_sha256: String },
    #[error(
        "PUP {pup_sha256} re-extracted kernel digest {extracted}, not its recorded digest {recorded}"
    )]
    KernelDigestConflict {
        pup_sha256: String,
        recorded: String,
        extracted: String,
    },
    #[error("existing {table}.tsv row names PUP {pup_sha256}, which is absent from pup.tsv")]
    ExistingPupReference {
        table: &'static str,
        pup_sha256: String,
    },
    #[error("existing extracted row names PUP {pup_sha256}, which is absent from kernel.tsv")]
    ExistingKernelReference { pup_sha256: String },
    #[error(
        "existing gate.tsv rows for PUP {pup_sha256} hash to {extracted}, not kernel.tsv digest {recorded}"
    )]
    ExistingGateDigest {
        pup_sha256: String,
        recorded: String,
        extracted: String,
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

#[derive(Debug, Default)]
struct ExistingRows {
    kernels: Vec<KernelRow>,
    stubs: Vec<StubRow>,
    subentries: Vec<SubentryRow>,
    gates: Vec<GateRow>,
}

fn emit(args: &Lv2CensusArgs, elf: &[u8]) -> Result<EmitSummary, Lv2CensusError> {
    let pups = compiled_pups()?;
    let pup = pups
        .iter()
        .find(|row| row.pup_sha256 == args.pup_sha256)
        .ok_or_else(|| Lv2CensusError::UnknownPup {
            pup_sha256: args.pup_sha256.clone(),
        })?;
    if pup.fw != args.fw {
        return Err(Lv2CensusError::FirmwareMismatch {
            pup_sha256: args.pup_sha256.clone(),
            recorded: pup.fw.clone(),
            requested: args.fw.clone(),
        });
    }

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
    let new_subentry_text =
        archive::subentry_tsv(&new_subentries).map_err(|source| Lv2CensusError::Render {
            table: SUBENTRY.name,
            source,
        })?;
    let new_subentry_count = new_subentries.len();
    let gate_classification =
        lv2_gate::classify(elf, classification).map_err(Lv2SubdispatchError::from)?;
    let new_gates = build_gate_rows(&args.pup_sha256, &gate_classification);
    let new_gate_text = archive::gate_tsv(&new_gates).map_err(|source| Lv2CensusError::Render {
        table: CAPABILITY_GATE.name,
        source,
    })?;
    let gated_count = new_gates
        .iter()
        .filter(|row| row.state == GateState::Gated)
        .count();
    let mut existing = load_existing(&args.output_dir)?;
    validate_existing(
        &existing.kernels,
        &existing.stubs,
        &existing.subentries,
        &existing.gates,
        &pups,
    )?;
    let new_kernel = KernelRow {
        pup_sha256: args.pup_sha256.clone(),
        kernel_elf_sha256: sha256_hex(elf),
        table_base: classification.discovery.table_vaddr,
        entry_width: classification.discovery.entry_width,
        entry_format: classification.discovery.entry_format.as_str().to_string(),
        entry_count: classification.discovery.entry_count,
        discovery_method: classification.discovery.method.as_str().to_string(),
        confidence: classification.discovery.confidence.as_str().to_string(),
        census_sha256: census_sha256.clone(),
        subentry_sha256: sha256_hex(new_subentry_text.as_bytes()),
        gate_sha256: sha256_hex(new_gate_text.as_bytes()),
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
    refuse_extraction_conflict(
        &args.pup_sha256,
        &existing,
        &new_kernel,
        &new_stubs,
        &new_subentries,
        &new_gates,
        args.replace_version,
    )?;
    let removed_pups = if args.replace_version {
        remove_version_rows(&args.fw, &args.pup_sha256, &pups, &mut existing)
    } else {
        0
    };
    existing
        .kernels
        .retain(|row| row.pup_sha256 != args.pup_sha256);
    existing
        .stubs
        .retain(|row| row.pup_sha256 != args.pup_sha256);
    existing
        .subentries
        .retain(|row| row.pup_sha256 != args.pup_sha256);
    existing
        .gates
        .retain(|row| row.pup_sha256 != args.pup_sha256);
    existing.kernels.push(new_kernel);
    existing.stubs.extend(new_stubs);
    existing.subentries.extend(new_subentries);
    existing.gates.extend(new_gates);

    refuse_digest_conflict(&existing.kernels, &pups)?;
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
        let selector_slot = format!("r{}", selector_slot + 3);
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
                        Lv2GateRead::ControlFlags1(mask) => {
                            format!("ctrl_flags1_0x{mask:08x}")
                        }
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

fn remove_version_rows(
    fw: &str,
    selected_pup: &str,
    pups: &[PupRow],
    existing: &mut ExistingRows,
) -> usize {
    let replaced_pups: BTreeSet<&str> = pups
        .iter()
        .filter(|row| row.fw == fw)
        .map(|row| row.pup_sha256.as_str())
        .collect();
    let removed = existing
        .kernels
        .iter()
        .filter(|row| {
            row.pup_sha256 != selected_pup && replaced_pups.contains(row.pup_sha256.as_str())
        })
        .count();
    existing
        .kernels
        .retain(|row| !replaced_pups.contains(row.pup_sha256.as_str()));
    existing
        .stubs
        .retain(|row| !replaced_pups.contains(row.pup_sha256.as_str()));
    existing
        .subentries
        .retain(|row| !replaced_pups.contains(row.pup_sha256.as_str()));
    existing
        .gates
        .retain(|row| !replaced_pups.contains(row.pup_sha256.as_str()));
    removed
}

fn refuse_extraction_conflict(
    pup_sha256: &str,
    existing: &ExistingRows,
    new_kernel: &KernelRow,
    new_stubs: &[StubRow],
    new_subentries: &[SubentryRow],
    new_gates: &[GateRow],
    allow_movement: bool,
) -> Result<(), Lv2CensusError> {
    let Some(existing_kernel) = existing
        .kernels
        .iter()
        .find(|row| row.pup_sha256 == pup_sha256)
    else {
        return Ok(());
    };
    if existing_kernel.kernel_elf_sha256 != new_kernel.kernel_elf_sha256 {
        return Err(Lv2CensusError::KernelDigestConflict {
            pup_sha256: pup_sha256.to_string(),
            recorded: existing_kernel.kernel_elf_sha256.clone(),
            extracted: new_kernel.kernel_elf_sha256.clone(),
        });
    }
    if allow_movement {
        return Ok(());
    }
    let mut existing_stubs: Vec<&StubRow> = existing
        .stubs
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_stubs.sort_by_key(|row| row.descriptor);
    let mut replacement_stubs: Vec<&StubRow> = new_stubs.iter().collect();
    replacement_stubs.sort_by_key(|row| row.descriptor);
    let mut existing_subentries: Vec<&SubentryRow> = existing
        .subentries
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_subentries.sort_by_key(|row| (row.ordinal, row.packet));
    let mut replacement_subentries: Vec<&SubentryRow> = new_subentries.iter().collect();
    replacement_subentries.sort_by_key(|row| (row.ordinal, row.packet));
    let mut existing_gates: Vec<&GateRow> = existing
        .gates
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_gates.sort_by_key(|row| row.ordinal);
    let mut replacement_gates: Vec<&GateRow> = new_gates.iter().collect();
    replacement_gates.sort_by_key(|row| row.ordinal);
    if existing_kernel != new_kernel
        || existing_stubs != replacement_stubs
        || existing_subentries != replacement_subentries
        || existing_gates != replacement_gates
    {
        return Err(Lv2CensusError::ExtractionConflict {
            pup_sha256: pup_sha256.to_string(),
        });
    }
    Ok(())
}

fn compiled_pups() -> Result<Vec<PupRow>, Lv2CensusError> {
    let table = archive::parse(&PUP, PUP_TSV).map_err(Lv2CensusError::PupTable)?;
    Ok(archive::pup_rows(&table))
}

fn load_existing(output_dir: &Path) -> Result<ExistingRows, Lv2CensusError> {
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
        (false, false, false, false) => Ok(ExistingRows::default()),
        (true, true, true, true) => {
            let kernel = read_table(&kernel_path, &KERNEL)?;
            let stub = read_table(&stub_path, &STUB)?;
            let subentry = archive::subentry_rows(&read_table(&subentry_path, &SUBENTRY)?);
            let gate = archive::gate_rows(&read_table(&gate_path, &CAPABILITY_GATE)?);
            Ok(ExistingRows {
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

fn refuse_digest_conflict(kernels: &[KernelRow], pups: &[PupRow]) -> Result<(), Lv2CensusError> {
    let firmware_by_pup: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let mut digest_by_firmware = BTreeMap::new();
    for kernel in kernels {
        let fw = firmware_by_pup
            .get(kernel.pup_sha256.as_str())
            .ok_or_else(|| Lv2CensusError::ExistingPupReference {
                table: KERNEL.name,
                pup_sha256: kernel.pup_sha256.clone(),
            })?;
        if digest_by_firmware
            .insert(*fw, kernel.census_sha256.as_str())
            .is_some_and(|digest| digest != kernel.census_sha256)
        {
            return Err(Lv2CensusError::DigestConflict {
                fw: (*fw).to_string(),
            });
        }
    }
    Ok(())
}

fn validate_existing(
    kernels: &[KernelRow],
    stubs: &[StubRow],
    subentries: &[SubentryRow],
    gates: &[GateRow],
    pups: &[PupRow],
) -> Result<(), Lv2CensusError> {
    let valid_pups: BTreeSet<&str> = pups.iter().map(|row| row.pup_sha256.as_str()).collect();
    let kernel_pups: BTreeSet<&str> = kernels.iter().map(|row| row.pup_sha256.as_str()).collect();
    if let Some(kernel) = kernels
        .iter()
        .find(|row| !valid_pups.contains(row.pup_sha256.as_str()))
    {
        return Err(Lv2CensusError::ExistingPupReference {
            table: KERNEL.name,
            pup_sha256: kernel.pup_sha256.clone(),
        });
    }
    if let Some(stub) = stubs
        .iter()
        .find(|row| !kernel_pups.contains(row.pup_sha256.as_str()))
    {
        return Err(Lv2CensusError::ExistingKernelReference {
            pup_sha256: stub.pup_sha256.clone(),
        });
    }
    if let Some(subentry) = subentries
        .iter()
        .find(|row| !kernel_pups.contains(row.pup_sha256.as_str()))
    {
        return Err(Lv2CensusError::ExistingKernelReference {
            pup_sha256: subentry.pup_sha256.clone(),
        });
    }
    if let Some(gate) = gates
        .iter()
        .find(|row| !kernel_pups.contains(row.pup_sha256.as_str()))
    {
        return Err(Lv2CensusError::ExistingKernelReference {
            pup_sha256: gate.pup_sha256.clone(),
        });
    }
    for kernel in kernels {
        let pup_gates: Vec<GateRow> = gates
            .iter()
            .filter(|row| row.pup_sha256 == kernel.pup_sha256)
            .cloned()
            .collect();
        let text = archive::gate_tsv(&pup_gates).map_err(|source| Lv2CensusError::Render {
            table: CAPABILITY_GATE.name,
            source,
        })?;
        let extracted = sha256_hex(text.as_bytes());
        if extracted != kernel.gate_sha256 {
            return Err(Lv2CensusError::ExistingGateDigest {
                pup_sha256: kernel.pup_sha256.clone(),
                recorded: kernel.gate_sha256.clone(),
                extracted,
            });
        }
    }
    Ok(())
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
    if census_path.is_file() {
        let existing =
            std::fs::read_to_string(&census_path).map_err(|source| Lv2CensusError::Read {
                path: census_path.clone(),
                source,
            })?;
        if existing != census_text && !args.replace_version {
            return Err(Lv2CensusError::CensusConflict {
                fw: args.fw.clone(),
                path: census_path,
            });
        }
    }
    if !census_path.is_file() || args.replace_version {
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
