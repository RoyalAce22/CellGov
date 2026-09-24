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
/// The segments come from `cellgov_ppu::loader::read_pt_loads`.
/// Returns 0, and warns which check refused, for:
///
/// - any refusal of that reader: a short or non-ELF input, a header of
///   another class, byte order, version or machine, no or an extended
///   program-header count, a slot size that is not the ELF64 one, or a
///   table past end-of-file
/// - a segment whose end leaves the 32-bit effective-address space
///
/// An image with no segment in the range also returns 0, and warns
/// nothing.
pub(crate) fn elf_user_region_end(data: &[u8], sink: &dyn crate::BootSink) -> usize {
    let segments = match cellgov_ppu::loader::read_pt_loads(data) {
        Ok(segments) => segments,
        Err(error) => {
            sink.warn(&format!("elf_user_region_end: {error}; returning 0"));
            return 0;
        }
    };
    let mut max_end: usize = 0;
    for seg in segments {
        if seg.memsz == 0 {
            continue;
        }
        if !(0x0001_0000..0x1000_0000).contains(&seg.vaddr) {
            continue;
        }
        // A PS3 effective address is 32 bits. `cellgov_ppu::loader`
        // refuses a PT_LOAD whose vaddr + memsz overflows or leaves
        // that space, as `LoadError::SegmentOutOfRange`, and the same
        // pair reaches here. A wrapped sum reads as a lower floor than
        // the segment it came from. That would place the guest heap on
        // top of the title's own code.
        let Some(end) = seg
            .vaddr
            .checked_add(seg.memsz)
            .filter(|&e| e <= u64::from(u32::MAX) + 1)
        else {
            sink.warn(&format!(
                "elf_user_region_end: PT_LOAD[{}] at 0x{:016x} size 0x{:x} \
                 leaves the 32-bit effective-address space; returning 0",
                seg.index, seg.vaddr, seg.memsz
            ));
            return 0;
        };
        let end = end as usize;
        if end > max_end {
            max_end = end;
        }
    }
    max_end
}

/// Why the save of a boot-checkpoint observation or a boot summary failed.
///
/// Three variants return before the save creates any file:
///
/// - [`PtLoadEnum`](Self::PtLoadEnum)
/// - [`Region`](Self::Region)
/// - [`InvalidBootSummary`](Self::InvalidBootSummary)
///
/// Every other variant fails at or after file creation, so a partial
/// file may remain at the path.
#[derive(Debug, thiserror::Error)]
pub enum ObservationSaveError {
    /// Enumerating PT_LOAD segments from the ELF failed.
    #[error("failed to enumerate PT_LOAD: {source}")]
    PtLoadEnum {
        /// The loader's own account of the refusal.
        #[source]
        source: cellgov_ppu::loader::LoadError,
    },
    /// Creating the output file failed.
    #[error("create {path} failed: {source}")]
    CreateOutput {
        /// Where the JSON is written.
        path: String,
        /// What the create refused with.
        #[source]
        source: std::io::Error,
    },
    /// Serializing the observation to JSON failed.
    #[error("serialize failed: {0}")]
    Serialize(#[source] serde_json::Error),
    /// Writing the trailing newline to the output failed.
    #[error("trailing newline {path} failed: {source}")]
    TrailingNewline {
        /// Where the JSON is written.
        path: String,
        /// What the write refused with.
        #[source]
        source: std::io::Error,
    },
    /// Flushing the output writer failed.
    #[error("flush {path} failed: {source}")]
    Flush {
        /// Where the JSON is written.
        path: String,
        /// What the flush refused with.
        #[source]
        source: std::io::Error,
    },
    /// Constructing the BootSummary rejected the
    /// checkpoint/outcome/steps tuple.
    #[error("invalid boot summary: {0}")]
    InvalidBootSummary(#[source] cellgov_compare::BootSummaryError),
    /// A region the extractor refused, whether the manifest named it
    /// or it was a PT_LOAD default.
    #[error("{0}")]
    Region(#[from] cellgov_compare::RegionExtractError),
}

/// What one boot-checkpoint observation is built from.
pub struct ObservationInputs<'a> {
    /// Where the JSON is written.
    pub path: &'a str,
    /// The title image, read for its PT_LOAD segments when the caller
    /// named no regions.
    pub elf_data: &'a [u8],
    /// End-of-run memory, one snapshot per address space the run
    /// created.
    pub final_spaces: &'a cellgov_compare::SpaceSnapshots,
    /// The terminal state the run reached.
    pub outcome: cellgov_compare::BootOutcome,
    /// Steps the run retired to reach it.
    pub steps: usize,
    /// Regions from a `--observation-manifest` the caller parsed.
    pub manifest_regions: Option<&'a [cellgov_compare::RegionDescriptor]>,
    /// Captured `sys_tty_write` byte stream.
    pub tty_log: &'a [u8],
    /// The identity triple the observation embeds.
    pub identity: &'a cellgov_compare::RunIdentity,
    /// Where the save reports what it wrote.
    pub sink: &'a dyn crate::BootSink,
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
/// - [`ObservationSaveError::PtLoadEnum`] when no manifest named the
///   regions and `elf_data` enumerates no PT_LOAD table. The function
///   creates no file in that case.
/// - [`ObservationSaveError::Region`] when the extractor refuses a
///   region, manifest-named or PT_LOAD default. The function creates
///   no file in that case.
/// - Another [`ObservationSaveError`] variant on an I/O or
///   serialization failure.
pub fn save_boot_observation(inputs: ObservationInputs<'_>) -> Result<(), ObservationSaveError> {
    let ObservationInputs {
        path,
        elf_data,
        final_spaces,
        outcome,
        steps,
        manifest_regions,
        tty_log,
        identity,
        sink,
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
    let observation = cellgov_compare::observe_from_boot(
        final_spaces,
        outcome,
        steps,
        &regions,
        tty_log,
        identity.clone(),
    )?;
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
    sink.note(&format!(
        "observation: wrote {} regions covering {} bytes to {path}",
        observation.memory_regions.len(),
        observation
            .memory_regions
            .iter()
            .map(|r| r.data.len())
            .sum::<usize>(),
    ));
    Ok(())
}

/// What one boot summary is built from.
pub struct BootSummaryInputs<'a> {
    /// Where the JSON is written.
    pub path: &'a str,
    /// The title the run booted; it names the checkpoint the summary
    /// records.
    pub title: &'a super::manifest::TitleManifest,
    /// The terminal state the run reached.
    pub outcome: cellgov_compare::BootOutcome,
    /// Steps the run retired to reach it.
    pub steps: usize,
    /// Retired instructions one `step()` granted.
    pub step_budget: cellgov_time::Budget,
    /// Invariant breaks the LV2 host logged over the run.
    pub host_invariant_breaks: u64,
    /// The identity triple the summary embeds.
    pub identity: cellgov_compare::RunIdentity,
    /// Where the save reports what it wrote.
    pub sink: &'a dyn crate::BootSink,
}

/// Serialize a [`cellgov_compare::BootSummary`] to `path` as
/// pretty JSON.
///
/// # Errors
///
/// - [`ObservationSaveError::InvalidBootSummary`] when the
///   checkpoint/outcome pair is inconsistent or `steps * budget`
///   overflows `u64`. The function creates no file in that case.
/// - Another [`ObservationSaveError`] variant on an I/O or
///   serialization failure.
pub fn save_boot_summary_json(inputs: BootSummaryInputs<'_>) -> Result<(), ObservationSaveError> {
    let BootSummaryInputs {
        path,
        title,
        outcome,
        steps,
        step_budget,
        host_invariant_breaks,
        identity,
        sink,
    } = inputs;
    let mut summary = cellgov_compare::BootSummary::new_with_breaks(
        title.checkpoint_trigger().kind(),
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
    sink.note(&format!("boot-summary: wrote {path}"));
    Ok(())
}

#[cfg(test)]
#[path = "tests/observation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/observation_phentsize_tests.rs"]
mod phentsize_tests;

#[cfg(test)]
#[path = "tests/observation_refusal_tests.rs"]
mod refusal_tests;

#[cfg(test)]
#[path = "tests/observation_provisional_tests.rs"]
mod provisional_tests;

#[cfg(test)]
#[path = "tests/observation_prewrite_refusal_tests.rs"]
mod prewrite_refusal_tests;
