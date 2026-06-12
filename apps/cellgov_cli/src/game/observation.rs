//! Checkpoint observation capture for `run-game --save-observation`.
//!
//! `CheckpointManifest` shares the schema `bridges/rpcs3_to_observation/`
//! consumes so both runners read the same TOML when comparing runs.

use serde::Deserialize;

/// Region list in a checkpoint observation manifest.
#[derive(Debug, Deserialize)]
pub(super) struct CheckpointManifest {
    pub(super) regions: Vec<CheckpointRegion>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckpointRegion {
    pub(super) name: String,
    #[serde(deserialize_with = "de_hex_u64")]
    pub(super) addr: u64,
    #[serde(deserialize_with = "de_hex_u64")]
    pub(super) size: u64,
}

/// Highest end address of any PT_LOAD segment whose vaddr falls in
/// `[0x00010000, 0x10000000)`. Segments above that range share no
/// address space with `sys_memory_allocate` and do not advance the
/// allocator base.
///
/// Returns 0 with a distinct stderr line for each failure mode
/// (short input, bad magic, truncated phdr table, no user segments).
pub(super) fn elf_user_region_end(data: &[u8]) -> usize {
    use cellgov_ps3_abi::elf::PT_LOAD;
    fn u16_be(d: &[u8], o: usize) -> u16 {
        u16::from_be_bytes([d[o], d[o + 1]])
    }
    fn u32_be(d: &[u8], o: usize) -> u32 {
        u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
    }
    fn u64_be(d: &[u8], o: usize) -> u64 {
        u64::from_be_bytes([
            d[o],
            d[o + 1],
            d[o + 2],
            d[o + 3],
            d[o + 4],
            d[o + 5],
            d[o + 6],
            d[o + 7],
        ])
    }
    if data.len() < 64 {
        eprintln!(
            "elf_user_region_end: input too short for ELF64 header ({} bytes); returning 0",
            data.len()
        );
        return 0;
    }
    if data[0..4] != [0x7f, 0x45, 0x4c, 0x46] {
        eprintln!("elf_user_region_end: ELF magic mismatch; returning 0");
        return 0;
    }
    let phoff = u64_be(data, 32) as usize;
    let phentsize = u16_be(data, 54) as usize;
    let phnum = u16_be(data, 56) as usize;
    // Up-front bound check: a mid-scan `break` would silently truncate.
    let ph_table_end = phoff.saturating_add(phentsize.saturating_mul(phnum));
    if ph_table_end > data.len() {
        eprintln!(
            "elf_user_region_end: program header table (phoff=0x{phoff:x} phentsize={phentsize} phnum={phnum}) extends past end-of-file ({} bytes); returning 0",
            data.len()
        );
        return 0;
    }
    let mut max_end: usize = 0;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if u32_be(data, base) != PT_LOAD {
            continue;
        }
        let p_vaddr = u64_be(data, base + 16) as usize;
        let p_memsz = u64_be(data, base + 40) as usize;
        if p_memsz == 0 {
            continue;
        }
        if (0x0001_0000..0x1000_0000).contains(&p_vaddr) {
            let end = p_vaddr + p_memsz;
            if end > max_end {
                max_end = end;
            }
        }
    }
    max_end
}

fn de_hex_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    let trimmed = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(trimmed, 16).map_err(serde::de::Error::custom)
}

/// Why writing the boot-checkpoint observation JSON failed.
#[derive(Debug, thiserror::Error)]
pub enum ObservationSaveError {
    /// Reading the region manifest failed.
    #[error("read {path}: {source}")]
    ManifestRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Parsing the region manifest TOML failed.
    #[error("parse {path}: {source}")]
    ManifestParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    /// Enumerating PT_LOAD segments from the ELF failed.
    #[error("failed to enumerate PT_LOAD: {source}")]
    PtLoadEnum {
        #[source]
        source: cellgov_ppu::loader::LoadError,
    },
    /// Creating the output file failed.
    #[error("create {path} failed: {source}")]
    CreateOutput {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Serializing the observation to JSON failed.
    #[error("serialize failed: {0}")]
    Serialize(#[source] serde_json::Error),
    /// Writing the trailing newline to the output failed.
    #[error("trailing newline {path} failed: {source}")]
    TrailingNewline {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Flushing the output writer failed.
    #[error("flush {path} failed: {source}")]
    Flush {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Constructing the BootSummary rejected the
    /// checkpoint/outcome/steps tuple.
    #[error("invalid boot summary: {0}")]
    InvalidBootSummary(#[source] cellgov_compare::BootSummaryError),
}

/// Build a boot-checkpoint observation and write it as JSON.
///
/// Regions default to one per PT_LOAD segment, named
/// `seg{index}_{ro|rw}`. With `manifest_path`, regions come from
/// the TOML manifest instead -- cross-runner comparison relies on
/// both runners reading the same file for matching region names.
///
/// # Errors
///
/// Returns [`ObservationSaveError`] on any I/O, parse, or
/// serialization failure so the caller can translate it to a
/// non-zero exit.
pub(super) fn save_boot_observation(
    path: &str,
    elf_data: &[u8],
    final_memory: &[u8],
    outcome: cellgov_compare::BootOutcome,
    steps: usize,
    manifest_path: Option<&str>,
    tty_log: &[u8],
) -> Result<(), ObservationSaveError> {
    let regions: Vec<cellgov_compare::RegionDescriptor> = match manifest_path {
        Some(mp) => {
            let text = std::fs::read_to_string(mp).map_err(|source| {
                ObservationSaveError::ManifestRead {
                    path: mp.to_string(),
                    source,
                }
            })?;
            let manifest: CheckpointManifest =
                toml::from_str(&text).map_err(|source| ObservationSaveError::ManifestParse {
                    path: mp.to_string(),
                    source,
                })?;
            manifest
                .regions
                .into_iter()
                .map(|r| cellgov_compare::RegionDescriptor {
                    name: r.name,
                    addr: r.addr,
                    size: r.size,
                })
                .collect()
        }
        None => {
            let segments = cellgov_ppu::loader::pt_load_segments(elf_data)
                .map_err(|source| ObservationSaveError::PtLoadEnum { source })?;
            segments
                .iter()
                .map(|s| {
                    let kind = if s.writable { "rw" } else { "ro" };
                    cellgov_compare::RegionDescriptor {
                        name: format!("seg{}_{kind}", s.index),
                        addr: s.vaddr,
                        size: s.memsz,
                    }
                })
                .collect()
        }
    };
    let observation =
        cellgov_compare::observe_from_boot(final_memory, outcome, steps, &regions, tty_log);
    // Pretty-print matches `rpcs3_to_observation`'s shape so the two
    // observation files diff cleanly under line-diff tools.
    let file =
        std::fs::File::create(path).map_err(|source| ObservationSaveError::CreateOutput {
            path: path.to_string(),
            source,
        })?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &observation)
        .map_err(ObservationSaveError::Serialize)?;
    std::io::Write::flush(&mut writer).map_err(|source| ObservationSaveError::Flush {
        path: path.to_string(),
        source,
    })?;
    println!(
        "observation: wrote {} regions covering {} bytes to {path}",
        observation.memory_regions.len(),
        observation
            .memory_regions
            .iter()
            .map(|r| r.data.len())
            .sum::<usize>(),
    );
    Ok(())
}

/// Translate [`super::manifest::CheckpointTrigger`] to
/// [`cellgov_compare::CheckpointKind`].
fn checkpoint_to_kind(cp: super::manifest::CheckpointTrigger) -> cellgov_compare::CheckpointKind {
    match cp {
        super::manifest::CheckpointTrigger::ProcessExit => {
            cellgov_compare::CheckpointKind::ProcessExit
        }
        super::manifest::CheckpointTrigger::FirstRsxWrite => {
            cellgov_compare::CheckpointKind::FirstRsxWrite
        }
        super::manifest::CheckpointTrigger::Pc(addr) => cellgov_compare::CheckpointKind::Pc {
            addr: cellgov_mem::GuestAddr::new(addr),
        },
    }
}

/// Serialize a [`cellgov_compare::BootSummary`] to `path` as
/// pretty JSON.
///
/// # Errors
///
/// Returns `Err(message)` on any I/O or serialization failure, or
/// if the checkpoint/outcome pair is inconsistent (see
/// [`cellgov_compare::BootSummaryError`]).
pub(super) fn save_boot_summary_json(
    path: &str,
    title: &super::manifest::TitleManifest,
    outcome: cellgov_compare::BootOutcome,
    steps: usize,
    step_budget: cellgov_time::Budget,
    host_invariant_breaks: u64,
) -> Result<(), ObservationSaveError> {
    let summary = cellgov_compare::BootSummary::new_with_breaks(
        checkpoint_to_kind(title.checkpoint_trigger()),
        outcome,
        steps as u64,
        step_budget,
        host_invariant_breaks,
    )
    .map_err(ObservationSaveError::InvalidBootSummary)?;
    let file =
        std::fs::File::create(path).map_err(|source| ObservationSaveError::CreateOutput {
            path: path.to_string(),
            source,
        })?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &summary).map_err(ObservationSaveError::Serialize)?;
    std::io::Write::write_all(&mut writer, b"\n").map_err(|source| {
        ObservationSaveError::TrailingNewline {
            path: path.to_string(),
            source,
        }
    })?;
    std::io::Write::flush(&mut writer).map_err(|source| ObservationSaveError::Flush {
        path: path.to_string(),
        source,
    })?;
    println!("boot-summary: wrote {path}");
    Ok(())
}

#[cfg(test)]
#[path = "tests/observation_tests.rs"]
mod tests;
