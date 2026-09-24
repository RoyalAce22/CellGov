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
/// Returns 0, and warns which check refused, for:
///
/// - an input too short for the ELF64 header
/// - a bad magic
/// - a program-header slot size that is not the ELF64 one
/// - a program-header table past end-of-file
/// - a segment whose end leaves the 32-bit effective-address space
///
/// An image with no segment in the range also returns 0, and warns
/// nothing.
pub(crate) fn elf_user_region_end(data: &[u8], sink: &dyn crate::BootSink) -> usize {
    use cellgov_ps3_abi::format::elf::{ELF_PHENTSIZE, PT_LOAD};
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
        sink.warn(&format!(
            "elf_user_region_end: input too short for ELF64 header ({} bytes); returning 0",
            data.len()
        ));
        return 0;
    }
    if data[0..4] != [0x7f, 0x45, 0x4c, 0x46] {
        sink.warn("elf_user_region_end: ELF magic mismatch; returning 0");
        return 0;
    }
    let phoff = u64_be(data, 32) as usize;
    let phentsize = u16_be(data, 54) as usize;
    let phnum = u16_be(data, 56) as usize;
    // The slot reads below use the fixed ELF64 field offsets (p_type at
    // 0, p_vaddr at 16, p_memsz at 40). A declared slot size other than
    // the architected one puts those reads outside the slot the stride
    // names. On the last entry they then read past end-of-file, where
    // they panic instead of refusing. `cellgov_ppu::loader` raises
    // `LoadError::BadPhentsize` for the same value; its
    // `ph_slot_offset` rejects it instead of clamping.
    if phentsize != ELF_PHENTSIZE {
        sink.warn(&format!(
            "elf_user_region_end: program-header entry size {phentsize} is not the ELF64 \
             program header's {ELF_PHENTSIZE}; returning 0"
        ));
        return 0;
    }
    // Up-front bound check: a mid-scan `break` would silently truncate.
    let ph_table_end = phoff.saturating_add(phentsize.saturating_mul(phnum));
    if ph_table_end > data.len() {
        sink.warn(&format!(
            "elf_user_region_end: program header table (phoff=0x{phoff:x} phentsize={phentsize} phnum={phnum}) extends past end-of-file ({} bytes); returning 0",
            data.len()
        ));
        return 0;
    }
    let mut max_end: usize = 0;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if u32_be(data, base) != PT_LOAD {
            continue;
        }
        let p_vaddr = u64_be(data, base + 16);
        let p_memsz = u64_be(data, base + 40);
        if p_memsz == 0 {
            continue;
        }
        if !(0x0001_0000..0x1000_0000).contains(&p_vaddr) {
            continue;
        }
        // A PS3 effective address is 32 bits. `cellgov_ppu::loader`
        // refuses a PT_LOAD whose vaddr + memsz overflows or leaves
        // that space, as `LoadError::SegmentOutOfRange`, and the same
        // pair reaches here. A wrapped sum reads as a lower floor than
        // the segment it came from. That would place the guest heap on
        // top of the title's own code.
        let Some(end) = p_vaddr
            .checked_add(p_memsz)
            .filter(|&e| e <= u64::from(u32::MAX) + 1)
        else {
            sink.warn(&format!(
                "elf_user_region_end: PT_LOAD[{i}] at 0x{p_vaddr:016x} size 0x{p_memsz:x} \
                 leaves the 32-bit effective-address space; returning 0"
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
