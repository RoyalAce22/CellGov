//! `cellgov dev fixture-gen` -- regenerate the cross-runner triple
//! (`compare_report.txt`, `REPRODUCTION.md`,
//! `cross_runner_summary.json`) from two observations plus a title
//! manifest.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cellgov_compare::{
    classify, classify::ClassifierContext, summarize, BootOverrides, ByteParity, Convergence,
    CrossRunnerSummary, DivergenceClass, Observation, ObservationCompareResult, ObservedOutcome,
    RegionPairOutcome, UnclassifiedRun, CODE_REGION_NAME, ELF_HEADER_SIZE,
};
use cellgov_ps3_abi::format::elf::ELF_MAGIC;

use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::FixtureGenArgs;
use super::self_load::{load_file, load_ppu_image_with_title};
use cellgov_boot::manifest::{CellKey, TitleManifest};

/// `ELF_HEADER_SIZE >= 58` is required for the `e_phnum` (56..58)
/// reads in [`elf_header_plus_phdr_table_end`] to be in bounds.
const _: () = assert!(ELF_HEADER_SIZE >= 58);

const COMPARE_REPORT_TEMPLATE: &str =
    include_str!("../../../../crates/cellgov_compare/templates/compare_report.txt.template");
const REPRODUCTION_TEMPLATE: &str = include_str!("templates/REPRODUCTION.md.template");

/// Max run length for inline `<hex> vs <hex>` byte listing in the
/// report. Longer runs render as head + tail + count.
const INLINE_BYTE_LIMIT: u64 = 16;

/// Path components between the workspace root and a cell's committed
/// fixture directory, `tests/fixtures/<id>/cross_runner/fw-<ver>/<game-ver>`.
const COMMITTED_CELL_DEPTH: usize = 6;

/// How many of those components a title's `<content-id>/` directory
/// covers.
const CONTENT_ID_DEPTH: usize = 3;

/// Why the ELF-header-plus-PHDR-table parser rejected the EBOOT.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ElfHeaderParseError {
    #[error(
        "EBOOT shorter than ELF64 header (got {len} bytes, need {})",
        ELF_HEADER_SIZE
    )]
    TooShort { len: usize },
    #[error("EBOOT magic is not 0x7f 'E' 'L' 'F' (got {:02x} {:02x} {:02x} {:02x})", found[0], found[1], found[2], found[3])]
    BadMagic { found: [u8; 4] },
    #[error("EBOOT EI_CLASS is not ELFCLASS64 (got {found})")]
    WrongClass { found: u8 },
    #[error("EBOOT EI_DATA is not ELFDATA2MSB (got {found})")]
    WrongEndian { found: u8 },
    #[error(
        "ELF PHDR table end overflows u64 (phoff={phoff}, phentsize={phentsize}, phnum={phnum})"
    )]
    PhdrTableOverflow {
        phoff: u64,
        phentsize: u64,
        phnum: u64,
    },
    #[error(
        "ELF PHDR table ends at {phdr_end} but the EBOOT is only {file_len} bytes \
         (phoff={phoff}, phentsize={phentsize}, phnum={phnum})"
    )]
    PhdrTableOutOfFile {
        phoff: u64,
        phentsize: u64,
        phnum: u64,
        phdr_end: u64,
        file_len: u64,
    },
}

/// Errors `dev fixture-gen` raises before the report writers.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureGenError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("serialize summary: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("ELF header: {0}")]
    ElfHeaderParse(#[from] ElfHeaderParseError),
    #[error("parse imports: {0}")]
    ImportParse(#[from] cellgov_ppu::prx::ImportParseError),
    /// `r.addr + phdr_end` would overflow `u64`. Reachable from
    /// user-provided observation JSON with `addr` near `u64::MAX`.
    #[error("code region addr 0x{addr:016x} + PHDR-table end 0x{phdr_end:016x} overflows u64")]
    CodeRegionAddrOverflow { addr: u64, phdr_end: u64 },
    #[error("sys_process_param addr 0x{addr:016x} + struct_size {struct_size} overflows u64")]
    SysProcParamAddrOverflow { addr: u64, struct_size: u64 },
}

/// Substitute `{{name}}` tokens in `template` with values from
/// `subs`. Unknown tokens are left in place. Single-pass: a value
/// containing `{{key}}` is not re-substituted.
pub(crate) fn apply_subs(template: &str, subs: &[(&str, &str)]) -> String {
    let map: BTreeMap<&str, &str> = subs.iter().copied().collect();
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        match after_open.find("}}") {
            Some(end) => {
                let key = &after_open[..end];
                match map.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(key);
                        out.push_str("}}");
                    }
                }
                rest = &after_open[end + 2..];
            }
            None => {
                out.push_str("{{");
                out.push_str(after_open);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The refusal for a CellGov capture taken under a boot override.
///
/// The fixture takes its identity from the store's composition, which
/// names no override. Without this refusal, the fixture files an
/// overridden run as a clean one.
fn overridden_capture_refusal(path: &str, overrides: &BootOverrides) -> Option<String> {
    if overrides.is_empty() {
        return None;
    }
    Some(format!(
        "fixture-gen: {path} was captured under boot override(s) {}; a committed \
         cross-runner result names the configuration its cell records, which applies none. \
         Re-capture with `boot run --save-observation` and no override flag",
        overrides.names().join(" ")
    ))
}

pub(crate) fn run(
    args: &FixtureGenArgs,
    vfs_flag: Option<&Path>,
) -> Result<CommandExitCode, CommandError> {
    let cellgov_path = args.cellgov.clone();
    let rpcs3_path = args.rpcs3.clone();
    let allow_divergence = args.allow_divergence;

    let manifest = TitleManifest::load_from_path(&args.manifest)
        .map_err(|error| CommandError::failed(format!("fixture-gen: load manifest: {error}")))?;
    let vfs_root = super::title::resolve_ps3_vfs_root(vfs_flag)?;
    // The fixture must name the EBOOT a boot run picks, so the
    // selection goes through the boot family's resolver.
    let composition = super::boot_cmd::resolve_composition(
        &args.selection,
        &vfs_root,
        &manifest,
        BootOverrides::default(),
    )?;
    let cell = super::boot_cmd::composed_cell(&composition).ok_or_else(|| {
        CommandError::failed(format!(
            "fixture-gen: {} composed no cell: a cross-runner result is filed under \
             (content id, firmware, game version), and this composition names none. An \
             unmanaged or absent firmware carries no version, a title the store does not \
             hold has no game-version axis, and an executable named by an absolute path \
             belongs to no firmware entry",
            manifest.name()
        ))
    })?;
    let fixtures = args
        .fixtures_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(crate::paths::DEFAULT_FIXTURES_DIR));
    let out_dir = crate::paths::cell_cross_runner_dir_in(&fixtures, &manifest.content_id, &cell);
    if !is_committed_tree(&fixtures) {
        eprintln!(
            "fixture-gen: --fixtures-dir {} is not the committed tree {}; REPRODUCTION.md's \
             links to the workspace root count the levels of the committed tree, so they \
             will not resolve from here",
            fixtures.display(),
            crate::paths::DEFAULT_FIXTURES_DIR,
        );
    }
    let eboot_path = manifest
        .resolve_eboot_in(&composition.eboot_dirs)
        .map_err(|error| CommandError::failed(format!("fixture-gen: resolve EBOOT: {error}")))?;
    let eboot_path = eboot_path.to_str().ok_or_else(|| {
        CommandError::failed(format!(
            "fixture-gen: EBOOT path {} has invalid UTF-8",
            eboot_path.display()
        ))
    })?;
    let eboot_bytes = load_ppu_image_with_title(eboot_path, &manifest, &vfs_root)?.elf_data;

    let cellgov_bytes = load_file(&cellgov_path)?;
    let cellgov: Observation = serde_json::from_slice(&cellgov_bytes).map_err(|error| {
        CommandError::failed(format!("fixture-gen: parse {cellgov_path}: {error}"))
    })?;
    if let Some(refusal) = overridden_capture_refusal(&cellgov_path, &cellgov.identity.overrides) {
        return Err(CommandError::failed(refusal));
    }
    let rpcs3_bytes = load_file(&rpcs3_path)?;
    let rpcs3: Observation = serde_json::from_slice(&rpcs3_bytes).map_err(|error| {
        CommandError::failed(format!("fixture-gen: parse {rpcs3_path}: {error}"))
    })?;

    // A zero-region non-Timeout observation would let the comparator
    // emit a confident verdict against empty data.
    if cellgov.memory_regions.is_empty() && !matches!(cellgov.outcome, ObservedOutcome::Timeout) {
        return Err(CommandError::failed(format!(
            "fixture-gen: CellGov observation at {cellgov_path} has zero \
             memory regions but reports outcome={}; the dump is incomplete. \
             Re-capture via `boot run --save-observation`.",
            cellgov.outcome,
        )));
    }
    if rpcs3.memory_regions.is_empty() && !matches!(rpcs3.outcome, ObservedOutcome::Timeout) {
        return Err(CommandError::failed(format!(
            "fixture-gen: RPCS3 observation at {rpcs3_path} has zero \
             memory regions but reports outcome={}; the dump is incomplete. \
             Re-capture per REPRODUCTION.md, or pass --outcome timeout to \
             the bridge if the RPCS3 run was actually capped.",
            rpcs3.outcome,
        )));
    }

    // The other runner's firmware comes from the capture, stamped when
    // the dump was converted. This command can run long after that
    // runner's installation changed, so a version read now would name a
    // library the dump never saw.
    let rpcs3_firmware = rpcs3.runner_firmware.clone().ok_or_else(|| {
        CommandError::failed(format!(
            "fixture-gen: {rpcs3_path} names no runner firmware, so nothing says which \
             library produced it. Re-convert the dump with `rpcs3_to_observation \
             --rpcs3-dir <dir>`"
        ))
    })?;

    let result = cellgov_compare::compare_observations(&cellgov, &rpcs3);
    let ctx = build_classifier_context(&eboot_bytes, &cellgov).map_err(|error| {
        CommandError::failed(format!("fixture-gen: build classifier context: {error}"))
    })?;
    let classes = classify_all(&result, &cellgov, &rpcs3, &ctx);
    let mut summary = summarize(&result, &classes)
        .with_firmware(composition.identity.clone(), rpcs3_firmware)
        .map_err(|error| CommandError::failed(format!("fixture-gen: {error}")))?;
    summary.oracle_gap_ordinals =
        oracle_gap_count(&vfs_root, &fixtures, &manifest.content_id, &cell);

    std::fs::create_dir_all(&out_dir).map_err(|error| {
        CommandError::failed(format!(
            "fixture-gen: create_dir_all {}: {e}",
            out_dir.display(),
            e = error,
        ))
    })?;

    write_compare_report(&out_dir, &result, &summary, &cellgov, &rpcs3)
        .map_err(|error| CommandError::failed(format!("fixture-gen: {error}")))?;
    write_reproduction(&out_dir, &manifest, &cell)
        .map_err(|error| CommandError::failed(format!("fixture-gen: {error}")))?;
    write_summary_json(&out_dir, &summary)
        .map_err(|error| CommandError::failed(format!("fixture-gen: {error}")))?;

    let (conv_str, parity_str) = summary.display_matrix_columns();
    println!(
        "fixture-gen: wrote cross-runner triple to {}: convergence={}, byte-parity={}",
        out_dir.display(),
        conv_str,
        parity_str,
    );

    if let Convergence::No { reason } = &summary.convergence {
        if allow_divergence {
            eprintln!(
                "fixture-gen: convergence failed ({reason}); --allow-divergence accepted, fixture committed to document the divergence"
            );
        } else {
            eprintln!(
                "fixture-gen: convergence failed ({reason}); pass --allow-divergence to commit a fixture documenting this state"
            );
            return Ok(CommandExitCode::new(exit_codes::FAILED));
        }
    }
    Ok(CommandExitCode::SUCCESS)
}

fn oracle_gap_count(
    vfs_root: &Path,
    fixtures: &Path,
    content_id: &str,
    cell: &CellKey,
) -> Option<u64> {
    let overlay = vfs_root.join(".cellgov/oracle-gap.tsv");
    let gap = std::fs::read_to_string(overlay).ok()?;
    let ordinals: std::collections::BTreeSet<u64> = gap
        .lines()
        .skip(2)
        .filter_map(|line| line.parse().ok())
        .collect();
    let anchor = crate::paths::boot_anchor_path_in(fixtures, content_id, cell);
    let summary: cellgov_compare::BootSummary =
        serde_json::from_str(&std::fs::read_to_string(anchor).ok()?).ok()?;
    Some(
        summary
            .unsupported_syscalls
            .keys()
            .filter(|ordinal| ordinals.contains(ordinal))
            .count() as u64,
    )
}

/// Build a [`ClassifierContext`] from EBOOT bytes + observation.
///
/// # Errors
///
/// `ElfHeaderParse` / `ImportParse` on malformed EBOOT structure;
/// `CodeRegionAddrOverflow` / `SysProcParamAddrOverflow` on input
/// addrs near `u64::MAX`.
pub(crate) fn build_classifier_context(
    eboot_bytes: &[u8],
    observation: &Observation,
) -> Result<ClassifierContext, FixtureGenError> {
    let elf_header_range = observation
        .memory_regions
        .iter()
        .find(|r| r.name == CODE_REGION_NAME)
        .map(|r| -> Result<Range<u64>, FixtureGenError> {
            let phdr_end = elf_header_plus_phdr_table_end(eboot_bytes)?;
            let end =
                r.addr
                    .checked_add(phdr_end)
                    .ok_or(FixtureGenError::CodeRegionAddrOverflow {
                        addr: r.addr,
                        phdr_end,
                    })?;
            Ok(r.addr..end)
        })
        .transpose()?;

    let sys_proc_param_range = match cellgov_ppu::loader::find_sys_process_param(eboot_bytes) {
        Some(p) => {
            let size = p.struct_size as u64;
            let end = p.guest_addr.checked_add(size).ok_or(
                FixtureGenError::SysProcParamAddrOverflow {
                    addr: p.guest_addr,
                    struct_size: size,
                },
            )?;
            Some(p.guest_addr..end)
        }
        None => None,
    };

    let hle_opd_ranges = compute_hle_opd_ranges(eboot_bytes)?;

    // sys_lwmutex_t handle-slot scan runs on the runtime snapshot (not
    // the EBOOT) because the lwmutex_free sentinel and attribute field
    // are only populated post-init. Every captured region is walked: a
    // title with several writable PT_LOADs keeps its lwmutexes in
    // whichever one the linker chose; the preamble match guards
    // against false positives.
    let mut sync_primitive_id_ranges: Vec<std::ops::Range<u64>> = observation
        .memory_regions
        .iter()
        .flat_map(|r| {
            cellgov_compare::sync_primitive_scan::find_sys_lwmutex_handle_slots(&r.data, r.addr)
        })
        .collect();
    // An lwcond names the lwmutex it binds, so its slots are found
    // against the lwmutex set of the same snapshot.
    let lwcond_slots: Vec<std::ops::Range<u64>> = observation
        .memory_regions
        .iter()
        .flat_map(|r| {
            cellgov_compare::sync_primitive_scan::find_sys_lwcond_handle_slots(
                &r.data,
                r.addr,
                &sync_primitive_id_ranges,
            )
        })
        .collect();
    sync_primitive_id_ranges.extend(lwcond_slots);

    let ctx = ClassifierContext {
        elf_header_range,
        sys_proc_param_range,
        hle_opd_ranges,
        sync_primitive_id_ranges,
    };
    ctx.debug_assert_disjoint();
    Ok(ctx)
}

/// One-past-the-end of the loaded ELF header + PHDR table in guest
/// memory. Returns at least [`ELF_HEADER_SIZE`].
///
/// # Errors
///
/// `TooShort`, `BadMagic`, `WrongClass` (must be ELFCLASS64),
/// `WrongEndian` (must be ELFDATA2MSB), `PhdrTableOverflow`,
/// `PhdrTableOutOfFile`.
fn elf_header_plus_phdr_table_end(eboot_bytes: &[u8]) -> Result<u64, ElfHeaderParseError> {
    if eboot_bytes.len() < ELF_HEADER_SIZE {
        return Err(ElfHeaderParseError::TooShort {
            len: eboot_bytes.len(),
        });
    }
    let magic: [u8; 4] = eboot_bytes[0..4]
        .try_into()
        .expect("guarded by len() >= ELF_HEADER_SIZE");
    if magic != ELF_MAGIC {
        return Err(ElfHeaderParseError::BadMagic { found: magic });
    }
    let class = eboot_bytes[4];
    if class != 2 {
        return Err(ElfHeaderParseError::WrongClass { found: class });
    }
    let endian = eboot_bytes[5];
    if endian != 2 {
        return Err(ElfHeaderParseError::WrongEndian { found: endian });
    }
    let phoff = u64::from_be_bytes(
        eboot_bytes[32..40]
            .try_into()
            .expect("guarded by len() >= ELF_HEADER_SIZE"),
    );
    let phentsize = u16::from_be_bytes(
        eboot_bytes[54..56]
            .try_into()
            .expect("guarded by len() >= ELF_HEADER_SIZE"),
    ) as u64;
    let phnum = u16::from_be_bytes(
        eboot_bytes[56..58]
            .try_into()
            .expect("guarded by len() >= ELF_HEADER_SIZE"),
    ) as u64;
    // phentsize * phnum cannot exceed u16::MAX * u16::MAX = 0xFFFE_0001,
    // safely inside u64. Only the phoff add can overflow.
    let tbl = phentsize * phnum;
    let phdr_end = phoff
        .checked_add(tbl)
        .ok_or(ElfHeaderParseError::PhdrTableOverflow {
            phoff,
            phentsize,
            phnum,
        })?;
    // The ELF specification places the program-header table inside
    // the file: `e_phnum` entries of `e_phentsize` bytes at file
    // offset `e_phoff`. A declared end past the image is a malformed
    // header. The returned value also becomes a classifier range that
    // marks every divergent byte under it non-semantic. A malformed
    // end would therefore claim guest bytes the ELF header does not
    // own. `disasm::elf::parse_pt_loads` rejects the same shape as
    // `PhdrOutOfFile`.
    let file_len = eboot_bytes.len() as u64;
    if phdr_end > file_len {
        return Err(ElfHeaderParseError::PhdrTableOutOfFile {
            phoff,
            phentsize,
            phnum,
            phdr_end,
            file_len,
        });
    }
    Ok(phdr_end.max(ELF_HEADER_SIZE as u64))
}

/// HLE-OPD-class slot ranges in the title's binary: one merged
/// range per maximal run of adjacent function-stub addresses, plus
/// one 4-byte range per variable-import `vref_addr`.
///
/// # Errors
///
/// `ImportParse` if `parse_imports` rejects the EBOOT. A parseable
/// EBOOT that legitimately imports nothing returns
/// `NoImportsTable`, which this function maps to an empty vec
/// rather than an error.
fn compute_hle_opd_ranges(eboot_bytes: &[u8]) -> Result<Vec<Range<u64>>, FixtureGenError> {
    let modules = match cellgov_ppu::prx::parse_imports(eboot_bytes) {
        Ok(m) => m,
        Err(cellgov_ppu::prx::ImportParseError::NoImportsTable) => return Ok(Vec::new()),
        Err(e) => return Err(FixtureGenError::ImportParse(e)),
    };

    let mut stubs: Vec<u32> = modules
        .iter()
        .flat_map(|m| m.functions.iter().map(|f| f.stub_addr))
        .collect();
    let mut ranges = merge_adjacent_stub_ranges(&mut stubs);

    let mut var_addrs: Vec<u32> = modules
        .iter()
        .flat_map(|m| m.variables.iter().map(|v| v.vref_addr))
        .collect();
    var_addrs.sort_unstable();
    var_addrs.dedup();
    for addr in var_addrs {
        // u32 cast bounds the arithmetic: u32::MAX + 4 fits in u64.
        debug_assert!((addr as u64).checked_add(4).is_some());
        ranges.push(addr as u64..addr as u64 + 4);
    }

    // Secondary OPD tables: adjacent tables collapse into one Range,
    // non-adjacent stay separate. Scan in
    // `cellgov_ppu::loader::find_secondary_opd_tables`.
    let secondary: Vec<Range<u64>> = cellgov_ppu::loader::find_secondary_opd_tables(eboot_bytes)
        .into_iter()
        .map(|t| t.guest_addr..t.guest_addr + t.size)
        .collect();
    let mut merged: Option<Range<u64>> = None;
    for r in secondary {
        merged = Some(match merged {
            Some(cur) if cur.end == r.start => cur.start..r.end,
            Some(cur) => {
                ranges.push(cur);
                r
            }
            None => r,
        });
    }
    if let Some(r) = merged {
        ranges.push(r);
    }

    // Indirect OPD tables (12-byte (id, ptr, opd_slot) rows): each
    // table contributes its OPD slot at row offset
    // INDIRECT_OPD_TABLE_SLOT_OFFSET. Scan in
    // `cellgov_ppu::loader::find_indirect_opd_tables`.
    for table in cellgov_ppu::loader::find_indirect_opd_tables(eboot_bytes) {
        let row_count = table.size / cellgov_ppu::loader::INDIRECT_OPD_TABLE_STRIDE;
        for row in 0..row_count {
            let slot_start = table.guest_addr
                + row * cellgov_ppu::loader::INDIRECT_OPD_TABLE_STRIDE
                + cellgov_ppu::loader::INDIRECT_OPD_TABLE_SLOT_OFFSET;
            ranges.push(slot_start..slot_start + 4);
        }
    }

    Ok(ranges)
}

/// Sort, dedup, and merge 4-byte stub addresses into the smallest
/// set of non-overlapping ranges; abutting stubs merge.
fn merge_adjacent_stub_ranges(stubs: &mut Vec<u32>) -> Vec<Range<u64>> {
    stubs.sort_unstable();
    stubs.dedup();
    let mut ranges = Vec::new();
    let mut cur: Option<Range<u64>> = None;
    for s in stubs.iter() {
        debug_assert!((*s as u64).checked_add(4).is_some());
        let next = *s as u64..*s as u64 + 4;
        cur = Some(match cur {
            Some(r) if r.end == next.start => r.start..next.end,
            Some(r) => {
                ranges.push(r);
                next
            }
            None => next,
        });
    }
    if let Some(r) = cur {
        ranges.push(r);
    }
    ranges
}

/// One class per [`cellgov_compare::ByteDivergence`] in `result`,
/// in flatten order over regions and bytes-within-region.
///
/// `cellgov` must be the observation that seeded `ctx` via
/// [`build_classifier_context`]; `rpcs3` supplies the second image
/// the HleOpdSlot structural checks read.
pub(crate) fn classify_all(
    result: &ObservationCompareResult,
    cellgov: &Observation,
    rpcs3: &Observation,
    ctx: &ClassifierContext,
) -> Vec<DivergenceClass> {
    let mut classes = Vec::new();
    for pair in &result.region_compare.pairs {
        if let RegionPairOutcome::ByteDivergence {
            name, addr, bytes, ..
        } = pair
        {
            let a_region = cellgov.memory_regions.iter().find(|r| &r.name == name);
            debug_assert_eq!(
                a_region.map(|r| r.addr),
                Some(*addr),
                "ByteDivergence pair addr disagrees with cellgov observation; \
                 compare_observations IdentityMismatch invariant violated"
            );
            // A ByteDivergence pair exists only when both observations
            // carry the region; a miss here is the same broken-invariant
            // class as the addr mismatch above. The empty-slice fallback
            // fails closed (the slot checks refuse and the run lands in
            // Pending), but it must still announce itself in debug.
            let b_region = rpcs3.memory_regions.iter().find(|r| &r.name == name);
            debug_assert!(
                a_region.is_some() && b_region.is_some(),
                "ByteDivergence pair region {name:?} missing from an observation; \
                 compare_observations invariant violated"
            );
            let a_data: &[u8] = a_region.map(|r| r.data.as_slice()).unwrap_or(&[]);
            let b_data: &[u8] = b_region.map(|r| r.data.as_slice()).unwrap_or(&[]);
            for div in bytes {
                classes.push(classify(div, *addr, ctx, a_data, b_data));
            }
        }
    }
    classes
}

fn write_compare_report(
    out_dir: &Path,
    _result: &ObservationCompareResult,
    summary: &CrossRunnerSummary,
    cellgov: &Observation,
    rpcs3: &Observation,
) -> Result<(), FixtureGenError> {
    let (conv_str, parity_str) = summary.display_matrix_columns();
    let body = apply_subs(
        COMPARE_REPORT_TEMPLATE,
        &[
            ("convergence_line", &conv_str),
            ("byte_parity_line", &parity_str),
            (
                "summary_section",
                &render_summary_section(summary, cellgov, rpcs3),
            ),
        ],
    );
    let path = out_dir.join("compare_report.txt");
    std::fs::write(&path, body).map_err(|e| FixtureGenError::Io {
        context: format!("write {}", path.display()),
        source: e,
    })
}

fn render_summary_section(
    summary: &CrossRunnerSummary,
    cellgov: &Observation,
    rpcs3: &Observation,
) -> String {
    let mut out = String::new();
    match &summary.byte_parity {
        ByteParity::Equivalent => {
            out.push_str("Byte-identical: 0 divergent bytes.\n");
            render_summary_tail(&mut out, summary);
        }
        ByteParity::NonSemantic { bytes } => {
            out.push_str(&format!("Total non-semantic bytes: {bytes}\n"));
            render_summary_tail(&mut out, summary);
        }
        ByteParity::Pending {
            non_semantic_bytes,
            unclassified_bytes,
        } => {
            out.push_str(&format!(
                "Classified non-semantic bytes: {non_semantic_bytes}\n"
            ));
            out.push_str(&format!(
                "Pending bytes: {unclassified_bytes} across {} run(s)\n",
                summary.unclassified_runs.len(),
            ));
            if !summary.per_class_bytes.is_empty() {
                out.push_str("Per-class breakdown:\n");
                for (class, bytes) in &summary.per_class_bytes {
                    out.push_str(&format!("  {class}: {bytes} bytes\n"));
                }
            }
            if let Some((cls, ident, off)) = &summary.lowest_offset_class {
                out.push_str(&format!(
                    "Lowest-offset divergence: {cls} in region {}@0x{:x} at offset 0x{off:x}\n",
                    ident.name, ident.addr,
                ));
            }
            out.push_str("\nUnclassified runs:\n");
            for run in &summary.unclassified_runs {
                out.push_str(&render_unclassified_run(run, cellgov, rpcs3));
            }
        }
        ByteParity::Diverge { reason } => {
            out.push_str(&format!(
                "Byte parity is undefined: runners did not converge ({reason}).\n"
            ));
            out.push_str(
                "Refresh the observation pair at a shared deterministic checkpoint before \
                 byte-level analysis becomes meaningful.\n",
            );
        }
    }
    out
}

fn render_summary_tail(out: &mut String, summary: &CrossRunnerSummary) {
    if !summary.per_class_bytes.is_empty() {
        out.push_str("Per-class breakdown:\n");
        for (class, bytes) in &summary.per_class_bytes {
            out.push_str(&format!("  {class}: {bytes} bytes\n"));
        }
    }
    if let Some((cls, ident, off)) = &summary.lowest_offset_class {
        out.push_str(&format!(
            "Lowest-offset divergence: {cls} in region {}@0x{:x} at offset 0x{off:x}\n",
            ident.name, ident.addr,
        ));
    }
}

/// One line per unclassified run, locator `<region>@0x<offset>+<length>`
/// followed by inline bytes (short runs) or head + tail + count.
fn render_unclassified_run(
    run: &UnclassifiedRun,
    cellgov: &Observation,
    rpcs3: &Observation,
) -> String {
    let cellgov_bytes = region_slice(cellgov, &run.region_name, run.offset, run.length);
    let rpcs3_bytes = region_slice(rpcs3, &run.region_name, run.offset, run.length);
    match (cellgov_bytes, rpcs3_bytes) {
        (Some(a), Some(b)) if run.length <= INLINE_BYTE_LIMIT => {
            format!(
                "  {}@0x{:x}+{}: cellgov={} rpcs3={}\n",
                run.region_name,
                run.offset,
                run.length,
                hex_run(&a),
                hex_run(&b),
            )
        }
        (Some(a), Some(b)) => {
            let head = 8.min(a.len());
            let tail_start = a.len().saturating_sub(8);
            let a_head = hex_run(&a[..head]);
            let a_tail = hex_run(&a[tail_start..]);
            let b_head = hex_run(&b[..head]);
            let b_tail = hex_run(&b[tail_start..]);
            format!(
                "  {}@0x{:x}+{}: cellgov={}..{} rpcs3={}..{} ({} bytes)\n",
                run.region_name, run.offset, run.length, a_head, a_tail, b_head, b_tail, run.length,
            )
        }
        (Some(_), None) => format!(
            "  {}@0x{:x}+{}: (region missing in rpcs3 observation)\n",
            run.region_name, run.offset, run.length,
        ),
        (None, Some(_)) => format!(
            "  {}@0x{:x}+{}: (region missing in cellgov observation)\n",
            run.region_name, run.offset, run.length,
        ),
        (None, None) => format!(
            "  {}@0x{:x}+{}: (region missing in both observations)\n",
            run.region_name, run.offset, run.length,
        ),
    }
}

fn region_slice(obs: &Observation, region: &str, offset: u64, length: u64) -> Option<Vec<u8>> {
    let r = obs.memory_regions.iter().find(|r| r.name == region)?;
    let off = usize::try_from(offset).ok()?;
    let len = usize::try_from(length).ok()?;
    let end = off.checked_add(len)?;
    if end > r.data.len() {
        return None;
    }
    Some(r.data[off..end].to_vec())
}

fn hex_run(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Whether `fixtures` is the committed tree the reproduction's
/// workspace-root links are written for.
///
/// Two spellings name that tree:
///
/// - the relative default,
/// - the same tree under the compiled-in workspace root.
fn is_committed_tree(fixtures: &Path) -> bool {
    fixtures == Path::new(crate::paths::DEFAULT_FIXTURES_DIR)
        || fixtures == crate::paths::fixtures_dir(&crate::paths::workspace_root()).as_path()
}

/// The `../` chain from a cell's fixture directory to the workspace
/// root, and the chain to the `<content-id>/` directory holding the two
/// observations.
///
/// Both chains are written for the committed tree, where a cell sits at
/// `tests/fixtures/<id>/cross_runner/fw-<ver>/<game-ver>`. A
/// firmware-shipped title has no game-version level, so its cells sit
/// one level shallower.
///
/// The second chain counts only the levels of the cell itself, so it
/// holds under any `--fixtures-dir`. The first also counts the levels
/// of `tests/fixtures`.
fn relative_prefixes(cell: &CellKey) -> (String, String) {
    let depth = if cell.game_ver.is_some() {
        COMMITTED_CELL_DEPTH
    } else {
        COMMITTED_CELL_DEPTH - 1
    };
    ("../".repeat(depth), "../".repeat(depth - CONTENT_ID_DEPTH))
}

/// The `--fw` / `--game-ver` pair that reproduces this cell.
fn selection_flags(cell: &CellKey) -> String {
    match &cell.game_ver {
        Some(v) => format!("--fw {} --game-ver {v}", cell.fw),
        None => format!("--fw {}", cell.fw),
    }
}

/// Render the reproduction for one cell.
///
/// [`apply_subs`] leaves an unmatched token in place, so a template key
/// absent from this list reaches the reader verbatim.
fn reproduction_body(
    content_id: &str,
    display_name: &str,
    checkpoint_kind: &str,
    cell: &CellKey,
) -> String {
    let (repo_root_rel, observation_dir) = relative_prefixes(cell);
    let cell_dir = match &cell.game_ver {
        Some(v) => format!("fw-{}/{v}", cell.fw),
        None => format!("fw-{}", cell.fw),
    };
    apply_subs(
        REPRODUCTION_TEMPLATE,
        &[
            ("content_id", content_id),
            ("display_name", display_name),
            ("checkpoint_kind", checkpoint_kind),
            ("cell_label", &cell.label()),
            ("cell_dir", &cell_dir),
            ("selection_flags", &selection_flags(cell)),
            ("repo_root_rel", &repo_root_rel),
            ("observation_dir", &observation_dir),
        ],
    )
}

fn write_reproduction(
    out_dir: &Path,
    manifest: &TitleManifest,
    cell: &CellKey,
) -> Result<(), FixtureGenError> {
    let checkpoint_kind = manifest.checkpoint_trigger().as_cli_str();
    let body = reproduction_body(
        &manifest.content_id,
        manifest.display_name(),
        &checkpoint_kind,
        cell,
    );
    let path = out_dir.join("REPRODUCTION.md");
    std::fs::write(&path, body).map_err(|e| FixtureGenError::Io {
        context: format!("write {}", path.display()),
        source: e,
    })
}

fn write_summary_json(out_dir: &Path, summary: &CrossRunnerSummary) -> Result<(), FixtureGenError> {
    let mut body = serde_json::to_string_pretty(summary)
        .map_err(|e| FixtureGenError::Serialize { source: e })?;
    body.push('\n');
    let path = out_dir.join(crate::paths::CROSS_RUNNER_SUMMARY_FILE);
    std::fs::write(&path, body).map_err(|e| FixtureGenError::Io {
        context: format!("write {}", path.display()),
        source: e,
    })
}

#[cfg(test)]
#[path = "tests/fixture_gen_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fixture_gen_cell_tests.rs"]
mod cell_tests;

#[cfg(test)]
#[path = "tests/fixture_gen_override_tests.rs"]
mod override_tests;
