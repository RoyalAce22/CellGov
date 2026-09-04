//! Checkpoint observation capture for `boot run --save-observation`.
//!
//! The region manifest is `cellgov_compare::CheckpointManifest`, the
//! schema the RPCS3 bridge reads too, and the caller parses it before
//! the boot starts.

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

/// Why writing the boot-checkpoint observation JSON failed.
#[derive(Debug, thiserror::Error)]
pub enum ObservationSaveError {
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
    /// A manifest region names an address space this run never
    /// created.
    #[error(
        "region {region} names address space {space}, but this run created \
         only spaces {present:?}; a spawned child's space is numbered from 1 \
         in spawn order, so either the title never spawned or the manifest \
         names the wrong space"
    )]
    RegionSpaceMissing {
        region: String,
        space: u32,
        present: Vec<u32>,
    },
}

/// What one boot-checkpoint observation is built from.
pub(super) struct ObservationInputs<'a> {
    /// Where the JSON is written.
    pub path: &'a str,
    /// The title image, read for its PT_LOAD segments when the caller
    /// named no regions.
    pub elf_data: &'a [u8],
    /// End-of-run memory, one snapshot per address space the run
    /// created.
    pub final_spaces: &'a cellgov_compare::SpaceSnapshots,
    pub outcome: cellgov_compare::BootOutcome,
    pub steps: usize,
    /// Regions from a `--observation-manifest` the caller parsed.
    pub manifest_regions: Option<&'a [cellgov_compare::RegionDescriptor]>,
    /// Captured `sys_tty_write` byte stream.
    pub tty_log: &'a [u8],
    /// The triple this run was composed from.
    pub identity: &'a cellgov_compare::RunIdentity,
}

/// Build a boot-checkpoint observation and write it as JSON.
///
/// Regions default to one per PT_LOAD segment, named
/// `seg{index}_{ro|rw}`. With `manifest_regions`, the caller's parsed
/// manifest names them instead -- cross-runner comparison relies on
/// both runners reading the same file for matching region names.
///
/// # Errors
///
/// Returns [`ObservationSaveError`] on any I/O or serialization
/// failure, or when a manifest region names an address space the run
/// never created.
pub(super) fn save_boot_observation(
    inputs: ObservationInputs<'_>,
) -> Result<(), ObservationSaveError> {
    let ObservationInputs {
        path,
        elf_data,
        final_spaces,
        outcome,
        steps,
        manifest_regions,
        tty_log,
        identity,
    } = inputs;
    let regions: Vec<cellgov_compare::RegionDescriptor> = match manifest_regions {
        Some(named) => named.to_vec(),
        None => {
            let segments = cellgov_ppu::loader::pt_load_segments(elf_data)
                .map_err(|source| ObservationSaveError::PtLoadEnum { source })?;
            segments
                .iter()
                .map(|s| {
                    let kind = if s.writable { "rw" } else { "ro" };
                    cellgov_compare::RegionDescriptor {
                        name: format!("seg{}_{kind}", s.index),
                        space: cellgov_compare::AddressSpaceId::BOOT,
                        addr: s.vaddr,
                        size: s.memsz,
                    }
                })
                .collect()
        }
    };
    // The extractor zero-fills a region whose space is absent, and a
    // second CellGov run zero-fills it identically, so the only place
    // this misconfiguration can surface is here, before anything is
    // written.
    if let Some(r) = regions
        .iter()
        .find(|r| !final_spaces.contains_key(&r.space))
    {
        return Err(ObservationSaveError::RegionSpaceMissing {
            region: r.name.clone(),
            space: r.space.raw(),
            present: final_spaces.keys().map(|s| s.raw()).collect(),
        });
    }
    let observation = cellgov_compare::observe_from_boot(
        final_spaces,
        outcome,
        steps,
        &regions,
        tty_log,
        identity.clone(),
    );
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
    identity: cellgov_compare::RunIdentity,
) -> Result<(), ObservationSaveError> {
    let mut summary = cellgov_compare::BootSummary::new_with_breaks(
        checkpoint_to_kind(title.checkpoint_trigger()),
        outcome,
        steps as u64,
        step_budget,
        host_invariant_breaks,
    )
    .map_err(ObservationSaveError::InvalidBootSummary)?;
    summary.identity = identity;
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
