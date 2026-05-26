//! `cellgov_cli fixture-gen` -- regenerate the cross-runner fixture
//! triple (`compare_report.txt`, `REPRODUCTION.md`,
//! `cross_runner_summary.json`) from two observations plus a title
//! manifest.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use cellgov_compare::{
    classify, classify::ClassifierContext, summarize, ByteParity, Convergence, CrossRunnerSummary,
    DivergenceClass, Observation, ObservationCompareResult, ObservedOutcome, RegionPairOutcome,
    UnclassifiedRun, CODE_REGION_NAME, ELF_HEADER_SIZE,
};

use super::args::find_flag_value;
use super::exit::{die, load_file_or_die};
use super::title::resolve_ps3_vfs_root;
use crate::game::manifest::TitleManifest;

/// `ELF_HEADER_SIZE >= 58` is required for the `e_phnum` (56..58)
/// reads in [`elf_header_plus_phdr_table_end`] to be in bounds.
const _: () = assert!(ELF_HEADER_SIZE >= 58);

const COMPARE_REPORT_TEMPLATE: &str =
    include_str!("../../../../crates/cellgov_compare/templates/compare_report.txt.template");
const REPRODUCTION_TEMPLATE: &str = include_str!("templates/REPRODUCTION.md.template");

/// Max run length for inline `<hex> vs <hex>` byte listing in the
/// report. Longer runs render as head + tail + count.
const INLINE_BYTE_LIMIT: u64 = 16;

/// Why the ELF-header-plus-PHDR-table parser rejected the EBOOT.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ElfHeaderParseError {
    #[error("EBOOT shorter than ELF64 header (got {len} bytes, need 64)")]
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
}

/// Errors `fixture-gen` raises before the report writers; `run()` is
/// the boundary between typed errors and the process-exit `die()`.
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
    /// `r.addr + phdr_end` would overflow `u64`. fixture-gen reads
    /// user-provided observation JSON, so `addr` near `u64::MAX` is
    /// reachable input even though a well-formed PS3 EBOOT never
    /// approaches it.
    #[error("code region addr 0x{addr:016x} + PHDR-table end 0x{phdr_end:016x} overflows u64")]
    CodeRegionAddrOverflow { addr: u64, phdr_end: u64 },
    /// Same rationale as [`Self::CodeRegionAddrOverflow`], applied
    /// to `guest_addr + struct_size` in the sys_proc_param scan.
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

pub(crate) fn run(args: &[String]) {
    let manifest_path = find_flag_value(args, "--manifest")
        .unwrap_or_else(|| die("fixture-gen: --manifest <path> is required"));
    let cellgov_path = find_flag_value(args, "--cellgov")
        .unwrap_or_else(|| die("fixture-gen: --cellgov <path> is required"));
    let rpcs3_path = find_flag_value(args, "--rpcs3")
        .unwrap_or_else(|| die("fixture-gen: --rpcs3 <path> is required"));
    let output_dir = find_flag_value(args, "--output-dir")
        .unwrap_or_else(|| die("fixture-gen: --output-dir <path> is required"));
    let allow_divergence = args.iter().any(|a| a == "--allow-divergence");

    let manifest = TitleManifest::load_from_path(Path::new(&manifest_path))
        .unwrap_or_else(|e| die(&format!("fixture-gen: load manifest: {e}")));
    let vfs_root = resolve_ps3_vfs_root(args);
    let eboot_path = manifest
        .resolve_eboot(&vfs_root)
        .unwrap_or_else(|e| die(&format!("fixture-gen: resolve EBOOT: {e}")));
    let eboot_bytes = std::fs::read(&eboot_path).unwrap_or_else(|e| {
        die(&format!(
            "fixture-gen: read EBOOT {}: {e}",
            eboot_path.display()
        ))
    });

    let cellgov: Observation = serde_json::from_slice(&load_file_or_die(&cellgov_path))
        .unwrap_or_else(|e| die(&format!("fixture-gen: parse {cellgov_path}: {e}")));
    let rpcs3: Observation = serde_json::from_slice(&load_file_or_die(&rpcs3_path))
        .unwrap_or_else(|e| die(&format!("fixture-gen: parse {rpcs3_path}: {e}")));

    // A zero-region non-Timeout observation lets the comparator emit a
    // confident verdict against empty data; reject before that happens.
    if cellgov.memory_regions.is_empty() && !matches!(cellgov.outcome, ObservedOutcome::Timeout) {
        die(&format!(
            "fixture-gen: CellGov observation at {cellgov_path} has zero \
             memory regions but reports outcome={}; the dump is incomplete. \
             Re-capture via `run-game --save-observation`.",
            cellgov.outcome,
        ));
    }
    if rpcs3.memory_regions.is_empty() && !matches!(rpcs3.outcome, ObservedOutcome::Timeout) {
        die(&format!(
            "fixture-gen: RPCS3 observation at {rpcs3_path} has zero \
             memory regions but reports outcome={}; the dump is incomplete. \
             Re-capture per REPRODUCTION.md, or pass --outcome timeout to \
             the bridge if the RPCS3 run was actually capped.",
            rpcs3.outcome,
        ));
    }

    let result = cellgov_compare::compare_observations(&cellgov, &rpcs3);
    let ctx = build_classifier_context(&eboot_bytes, &cellgov)
        .unwrap_or_else(|e| die(&format!("fixture-gen: build classifier context: {e}")));
    let classes = classify_all(&result, &cellgov, &ctx);
    let summary = summarize(&result, &classes);

    let out_dir = Path::new(&output_dir);
    std::fs::create_dir_all(out_dir).unwrap_or_else(|e| {
        die(&format!(
            "fixture-gen: create_dir_all {}: {e}",
            out_dir.display()
        ))
    });

    write_compare_report(out_dir, &result, &summary, &cellgov, &rpcs3)
        .unwrap_or_else(|e| die(&format!("fixture-gen: {e}")));
    write_reproduction(out_dir, &manifest).unwrap_or_else(|e| die(&format!("fixture-gen: {e}")));
    write_summary_json(out_dir, &summary).unwrap_or_else(|e| die(&format!("fixture-gen: {e}")));

    let (conv_str, parity_str) = summary.display_matrix_columns();
    println!(
        "fixture-gen: wrote triple to {}: convergence={}, byte-parity={}",
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
            std::process::exit(1);
        }
    }
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

    let ctx = ClassifierContext {
        elf_header_range,
        sys_proc_param_range,
        hle_opd_ranges,
    };
    ctx.debug_assert_disjoint();
    Ok(ctx)
}

/// One-past-the-end of the loaded ELF header + PHDR table in guest
/// memory. PS3 EBOOTs map both via a PT_LOAD at `p_offset=0`, so the
/// PHDR bytes live inside the title's `code` region alongside the
/// 64-byte header and share its non-semantic-reconstruction class.
///
/// Returns at least [`ELF_HEADER_SIZE`].
///
/// # Errors
///
/// `TooShort`, `BadMagic`, `WrongClass` (must be ELFCLASS64),
/// `WrongEndian` (must be ELFDATA2MSB), `PhdrTableOverflow`.
fn elf_header_plus_phdr_table_end(eboot_bytes: &[u8]) -> Result<u64, ElfHeaderParseError> {
    if eboot_bytes.len() < ELF_HEADER_SIZE {
        return Err(ElfHeaderParseError::TooShort {
            len: eboot_bytes.len(),
        });
    }
    let magic: [u8; 4] = eboot_bytes[0..4]
        .try_into()
        .expect("guarded by len() >= ELF_HEADER_SIZE");
    if magic != [0x7f, b'E', b'L', b'F'] {
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
    Ok(phdr_end.max(ELF_HEADER_SIZE as u64))
}

/// HLE-OPD-class slot ranges in the title's binary: one merged
/// range per maximal run of adjacent function-stub addresses, plus
/// one 4-byte range per variable-import `vref_addr`. The function
/// stubs may or may not pack densely; the merge pass produces the
/// same shape under either layout without claiming density.
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

    // Secondary OPD tables. The PRX-link CRT0 walker patches these
    // tables with HLE OPDs from the same address space as the
    // primary import-stub table; the classifier rule is identical
    // (`HleOpdSlot` on containment). Adjacent tables collapse into
    // one Range; non-adjacent stay separate. See
    // `cellgov_ppu::loader::find_secondary_opd_tables` for the scan.
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

    // Indirect OPD tables (12-byte (id, ptr, opd_slot) rows). Each
    // table contributes its OPD slot at row offset
    // INDIRECT_OPD_TABLE_SLOT_OFFSET; classifier rule is the same
    // `HleOpdSlot` per-slot. See
    // `cellgov_ppu::loader::find_indirect_opd_tables` for the scan.
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
/// [`build_classifier_context`]. Each `ByteDivergence` pair's `addr`
/// equals the cellgov region's `addr` by `compare_observations`
/// construction (addrs disagree only via `IdentityMismatch`, which
/// this fn skips).
pub(crate) fn classify_all(
    result: &ObservationCompareResult,
    cellgov: &Observation,
    ctx: &ClassifierContext,
) -> Vec<DivergenceClass> {
    let mut classes = Vec::new();
    for pair in &result.region_compare.pairs {
        if let RegionPairOutcome::ByteDivergence {
            name, addr, bytes, ..
        } = pair
        {
            debug_assert_eq!(
                cellgov
                    .memory_regions
                    .iter()
                    .find(|r| &r.name == name)
                    .map(|r| r.addr),
                Some(*addr),
                "ByteDivergence pair addr disagrees with cellgov observation; \
                 compare_observations IdentityMismatch invariant violated"
            );
            for div in bytes {
                classes.push(classify(div, *addr, ctx));
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
/// Region-missing cases name which side lacked the region so the
/// cross-runner asymmetry is visible in the fixture.
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

fn write_reproduction(out_dir: &Path, manifest: &TitleManifest) -> Result<(), FixtureGenError> {
    let checkpoint_kind = manifest.checkpoint_trigger().as_cli_str();
    let body = apply_subs(
        REPRODUCTION_TEMPLATE,
        &[
            ("content_id", &manifest.content_id),
            ("display_name", manifest.display_name()),
            ("checkpoint_kind", &checkpoint_kind),
        ],
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
    let path = out_dir.join("cross_runner_summary.json");
    std::fs::write(&path, body).map_err(|e| FixtureGenError::Io {
        context: format!("write {}", path.display()),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_compare::{
        compare_observations, NamedMemoryRegion, ObservationMetadata, ObservedOutcome,
    };

    fn obs(outcome: ObservedOutcome, regions: Vec<NamedMemoryRegion>) -> Observation {
        Observation {
            outcome,
            memory_regions: regions,
            events: Vec::new(),
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: "test".to_string(),
                steps: Some(1),
            },
            tty_log: Vec::new(),
        }
    }

    fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
        NamedMemoryRegion {
            name: name.to_string(),
            addr,
            data,
        }
    }

    /// Synthetic 64-byte ELF64 BE PPC header (passes the parser's
    /// magic / class / endian gate; `phoff` / `phentsize` / `phnum`
    /// caller-supplied so tests can drive the PHDR-table end).
    fn synthetic_elf64_be(phoff: u64, phentsize: u16, phnum: u16) -> Vec<u8> {
        let mut eboot = vec![0u8; 64];
        eboot[0..4].copy_from_slice(b"\x7fELF");
        eboot[4] = 2; // ELFCLASS64
        eboot[5] = 2; // ELFDATA2MSB
        eboot[32..40].copy_from_slice(&phoff.to_be_bytes());
        eboot[54..56].copy_from_slice(&phentsize.to_be_bytes());
        eboot[56..58].copy_from_slice(&phnum.to_be_bytes());
        eboot
    }

    #[test]
    fn apply_subs_replaces_named_tokens() {
        let out = apply_subs(
            "Hello {{name}} -- you are {{role}}",
            &[("name", "World"), ("role", "tester")],
        );
        assert_eq!(out, "Hello World -- you are tester");
    }

    #[test]
    fn apply_subs_leaves_unknown_tokens_visible() {
        let out = apply_subs("Stale: {{ghost}}", &[("ignored", "value")]);
        assert_eq!(out, "Stale: {{ghost}}");
    }

    #[test]
    fn apply_subs_is_deterministic_regardless_of_slice_order() {
        let a = apply_subs("{{a}}/{{b}}", &[("a", "1"), ("b", "2")]);
        let b = apply_subs("{{a}}/{{b}}", &[("b", "2"), ("a", "1")]);
        assert_eq!(a, b);
    }

    #[test]
    fn apply_subs_does_not_re_substitute_into_value() {
        let out = apply_subs("[{{a}}]", &[("a", "{{b}}"), ("b", "EVIL")]);
        assert_eq!(out, "[{{b}}]");
    }

    #[test]
    fn apply_subs_handles_unterminated_token() {
        let out = apply_subs("trailing {{open", &[("open", "X")]);
        assert_eq!(out, "trailing {{open");
    }

    #[test]
    fn build_classifier_context_populates_elf_header_when_code_region_present() {
        let observation = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
        );
        let eboot = synthetic_elf64_be(0, 0, 0);
        let ctx = build_classifier_context(&eboot, &observation).unwrap();
        assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
    }

    #[test]
    fn elf_header_range_widens_to_include_phdr_table() {
        // phoff=0x40 + 5 * phentsize=0x38 -> PHDR end at 0x158.
        let eboot = synthetic_elf64_be(0x40, 0x38, 5);
        assert_eq!(elf_header_plus_phdr_table_end(&eboot).unwrap(), 0x158);
    }

    #[test]
    fn elf_header_plus_phdr_helper_rejects_short_input() {
        assert!(matches!(
            elf_header_plus_phdr_table_end(&[0u8; 32]),
            Err(ElfHeaderParseError::TooShort { len: 32 })
        ));
    }

    #[test]
    fn elf_header_plus_phdr_helper_rejects_bad_magic() {
        let mut eboot = synthetic_elf64_be(0, 0, 0);
        eboot[0] = 0xCC;
        assert!(matches!(
            elf_header_plus_phdr_table_end(&eboot),
            Err(ElfHeaderParseError::BadMagic { .. })
        ));
    }

    #[test]
    fn elf_header_plus_phdr_helper_rejects_elf_class_32() {
        let mut eboot = synthetic_elf64_be(0, 0, 0);
        eboot[4] = 1; // ELFCLASS32
        assert!(matches!(
            elf_header_plus_phdr_table_end(&eboot),
            Err(ElfHeaderParseError::WrongClass { found: 1 })
        ));
    }

    #[test]
    fn elf_header_plus_phdr_helper_rejects_little_endian() {
        let mut eboot = synthetic_elf64_be(0, 0, 0);
        eboot[5] = 1; // ELFDATA2LSB
        assert!(matches!(
            elf_header_plus_phdr_table_end(&eboot),
            Err(ElfHeaderParseError::WrongEndian { found: 1 })
        ));
    }

    #[test]
    fn elf_header_plus_phdr_helper_rejects_phdr_overflow() {
        let eboot = synthetic_elf64_be(u64::MAX, u16::MAX, u16::MAX);
        assert!(matches!(
            elf_header_plus_phdr_table_end(&eboot),
            Err(ElfHeaderParseError::PhdrTableOverflow { .. })
        ));
    }

    #[test]
    fn build_classifier_context_with_no_code_region_leaves_header_none() {
        let observation = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0u8; 4])],
        );
        let eboot = synthetic_elf64_be(0, 0, 0);
        let ctx = build_classifier_context(&eboot, &observation).unwrap();
        assert!(ctx.elf_header_range.is_none());
    }

    #[test]
    fn build_classifier_context_propagates_elf_parse_error() {
        let observation = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
        );
        let eboot = vec![0u8; 32];
        assert!(matches!(
            build_classifier_context(&eboot, &observation),
            Err(FixtureGenError::ElfHeaderParse(
                ElfHeaderParseError::TooShort { len: 32 }
            ))
        ));
    }

    #[test]
    fn compute_hle_opd_ranges_no_imports_table_is_empty_vec() {
        let eboot = synthetic_elf64_be(0, 0, 0);
        assert_eq!(
            compute_hle_opd_ranges(&eboot).unwrap(),
            Vec::<Range<u64>>::new()
        );
    }

    #[test]
    fn compute_hle_opd_ranges_propagates_non_no_imports_table_errors() {
        let eboot = vec![0u8; 32];
        match compute_hle_opd_ranges(&eboot) {
            Err(FixtureGenError::ImportParse(e)) => assert!(
                !matches!(e, cellgov_ppu::prx::ImportParseError::NoImportsTable),
                "NoImportsTable must be mapped to Ok(vec![]); got Err propagation"
            ),
            other => panic!("expected ImportParse error, got {other:?}"),
        }
    }

    #[test]
    fn merge_adjacent_stub_ranges_empty_input_returns_empty() {
        let mut stubs: Vec<u32> = vec![];
        assert!(merge_adjacent_stub_ranges(&mut stubs).is_empty());
    }

    #[test]
    fn merge_adjacent_stub_ranges_single_stub_one_range() {
        let mut stubs = vec![0x10_0000u32];
        let ranges = merge_adjacent_stub_ranges(&mut stubs);
        assert_eq!(ranges, vec![0x10_0000u64..0x10_0004u64]);
    }

    #[test]
    fn merge_adjacent_stub_ranges_two_adjacent_merge_to_one() {
        let mut stubs = vec![0x10_0000u32, 0x10_0004u32];
        let ranges = merge_adjacent_stub_ranges(&mut stubs);
        assert_eq!(ranges, vec![0x10_0000u64..0x10_0008u64]);
    }

    #[test]
    fn merge_adjacent_stub_ranges_two_non_adjacent_stay_two() {
        let mut stubs = vec![0x10_0000u32, 0x10_0010u32];
        let ranges = merge_adjacent_stub_ranges(&mut stubs);
        assert_eq!(
            ranges,
            vec![0x10_0000u64..0x10_0004u64, 0x10_0010u64..0x10_0014u64]
        );
    }

    #[test]
    fn merge_adjacent_stub_ranges_unsorted_with_dupes_sorts_and_dedups() {
        let mut stubs = vec![
            0x10_0008u32,
            0x10_0000u32,
            0x10_0004u32,
            0x10_0000u32,
            0x10_0010u32,
        ];
        let ranges = merge_adjacent_stub_ranges(&mut stubs);
        assert_eq!(
            ranges,
            vec![0x10_0000u64..0x10_000Cu64, 0x10_0010u64..0x10_0014u64]
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "overlap")]
    fn classifier_context_overlap_panics_in_debug() {
        let ctx = ClassifierContext {
            elf_header_range: Some(0x1000..0x2000),
            sys_proc_param_range: Some(0x1500..0x2500),
            hle_opd_ranges: Vec::new(),
        };
        ctx.debug_assert_disjoint();
    }

    #[test]
    fn build_classifier_context_overflows_on_code_region_addr_near_u64_max() {
        let observation = obs(
            ObservedOutcome::Completed,
            vec![region("code", u64::MAX - 0x20, vec![0u8; 0x40])],
        );
        // phoff=0x40 + 5 * 0x38 = 0x158 PHDR end; adds to addr -> overflow.
        let eboot = synthetic_elf64_be(0x40, 0x38, 5);
        assert!(matches!(
            build_classifier_context(&eboot, &observation),
            Err(FixtureGenError::CodeRegionAddrOverflow { .. })
        ));
    }

    /// Synthetic EBOOT with a single PT_LOAD covering a
    /// sys_proc_param magic struct at file offset 0x100. Caller
    /// supplies `p_vaddr` and `struct_size`.
    fn synthetic_eboot_with_sys_proc_param_at(p_vaddr: u64, struct_size: u32) -> Vec<u8> {
        use cellgov_ps3_abi::elf::{PT_LOAD, SYS_PROCESS_PARAM_MAGIC};
        let phoff: usize = 64;
        let phentsize: usize = 56;
        let pt_load_offset: usize = 0x100;
        let pt_load_size: usize = 0x40;
        let payload_offset: usize = pt_load_offset; // struct starts here
        let total = payload_offset + pt_load_size + 32;
        let mut data = vec![0u8; total];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 2;
        data[5] = 2;
        data[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
        data[54..56].copy_from_slice(&(phentsize as u16).to_be_bytes());
        data[56..58].copy_from_slice(&1u16.to_be_bytes());
        data[phoff..phoff + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[phoff + 8..phoff + 16].copy_from_slice(&(pt_load_offset as u64).to_be_bytes());
        data[phoff + 16..phoff + 24].copy_from_slice(&p_vaddr.to_be_bytes());
        data[phoff + 32..phoff + 40].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        data[phoff + 40..phoff + 48].copy_from_slice(&(pt_load_size as u64).to_be_bytes());
        let start = payload_offset;
        data[start..start + 4].copy_from_slice(&struct_size.to_be_bytes());
        data[start + 4..start + 8].copy_from_slice(&SYS_PROCESS_PARAM_MAGIC.to_be_bytes());
        data
    }

    #[test]
    fn build_classifier_context_overflows_on_sys_proc_param_addr_near_u64_max() {
        // Positive control so the overflow assertion below is not vacuous.
        let normal_eboot = synthetic_eboot_with_sys_proc_param_at(0x10_0000, 0x30);
        let normal_obs = obs(ObservedOutcome::Completed, vec![]);
        let normal_ctx = build_classifier_context(&normal_eboot, &normal_obs).unwrap();
        assert!(normal_ctx.sys_proc_param_range.is_some());

        let observation = obs(ObservedOutcome::Completed, vec![]);
        let eboot = synthetic_eboot_with_sys_proc_param_at(u64::MAX - 0x10, 0x30);
        assert!(matches!(
            build_classifier_context(&eboot, &observation),
            Err(FixtureGenError::SysProcParamAddrOverflow { .. })
        ));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "IdentityMismatch invariant violated")]
    fn classify_all_panics_on_addr_mismatch_in_debug() {
        use cellgov_compare::{
            ByteDivergence, EventCompare, RegionCompareSummary, StateHashCompare, StepCompare,
        };
        let cellgov = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
        );
        // Hand-built: compare_observations cannot legitimately emit an
        // addr-disagreeing pair, so the debug_assert can only be reached
        // by constructing the shape directly.
        let result = ObservationCompareResult {
            outcome_match: true,
            a_outcome: ObservedOutcome::Completed,
            b_outcome: ObservedOutcome::Completed,
            region_compare: RegionCompareSummary {
                a_count: 1,
                b_count: 1,
                pairs: vec![RegionPairOutcome::ByteDivergence {
                    name: "code".to_string(),
                    addr: 0x20000, // != cellgov's 0x10000
                    length: 4,
                    bytes: vec![ByteDivergence {
                        offset: 0,
                        length: 1,
                        a_byte: 0,
                        b_byte: 0xFF,
                    }],
                }],
            },
            event_compare: EventCompare::Equal { count: 0 },
            state_hash_compare: StateHashCompare::NoHashInfo,
            step_compare: StepCompare::NoStepInfo,
            a_runner: "cellgov".to_string(),
            b_runner: "rpcs3".to_string(),
        };
        let _ = classify_all(&result, &cellgov, &ClassifierContext::default());
    }

    #[test]
    fn classify_all_returns_one_class_per_byte_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x17] = 0xAA;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
        );
        let result = compare_observations(&a, &b);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        let classes = classify_all(&result, &a, &ctx);
        assert_eq!(classes, vec![DivergenceClass::ElfHeader]);
    }

    #[test]
    fn classify_all_returns_unclassified_without_dying() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0u8; 8])],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0xFFu8; 8])],
        );
        let result = compare_observations(&a, &b);
        let classes = classify_all(&result, &a, &ClassifierContext::default());
        assert_eq!(classes, vec![DivergenceClass::Unclassified]);
    }

    // `DivergenceClass` Display strings are pinned upstream by
    // `divergence_class_display_strings_are_stable` in
    // crates/cellgov_compare/src/classify.rs; the consumer-side
    // `contains` checks below double-pin them.
    #[test]
    fn render_summary_section_non_semantic_lists_per_class_and_lowest_offset() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x17] = 0xAA;
        b_data[0x35] = 0xBB;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
        );
        let result = compare_observations(&a, &b);
        let summary = summarize(
            &result,
            &[DivergenceClass::ElfHeader, DivergenceClass::ElfHeader],
        );
        let section = render_summary_section(&summary, &a, &b);
        assert!(section.contains("Total non-semantic bytes: 2"));
        assert!(section.contains("ElfHeader: 2 bytes"));
        assert!(section.contains("Lowest-offset divergence: ElfHeader"));
        assert!(section.contains("code@0x10000"));
    }

    #[test]
    fn render_summary_section_pending_enumerates_runs_with_bytes() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, vec![0u8; 0x40]),
                region("data", 0x80000, vec![0x00u8; 4]),
            ],
        );
        let mut b_code = vec![0u8; 0x40];
        b_code[0x17] = 0xAA;
        let b_data = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, b_data),
            ],
        );
        let result = compare_observations(&a, &b);
        let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::Unclassified];
        let summary = summarize(&result, &classes);
        let section = render_summary_section(&summary, &a, &b);
        assert!(
            section.contains("Pending bytes: 4 across 1 run(s)"),
            "section missing pending header: {section}"
        );
        assert!(
            section.contains("data@0x0+4"),
            "section missing per-run locator: {section}"
        );
        assert!(
            section.contains("cellgov=00000000") && section.contains("rpcs3=aabbccdd"),
            "section missing inline bytes: {section}"
        );
    }

    #[test]
    fn render_summary_section_diverge_explains_undefined_byte_parity() {
        let a = obs(ObservedOutcome::Fault, vec![region("r", 0, vec![0u8; 4])]);
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0u8; 4])],
        );
        let result = compare_observations(&a, &b);
        let summary = summarize(&result, &[]);
        let section = render_summary_section(&summary, &a, &b);
        assert!(section.contains("Byte parity is undefined"));
        assert!(section.contains("did not converge"));
        assert!(section.contains("outcome: Fault vs Completed"));
    }

    #[test]
    fn render_summary_section_multiple_classes_are_byte_deterministic() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, vec![0u8; 0x40]),
                region("data", 0x80000, vec![0x00u8; 8]),
            ],
        );
        let mut b_code = vec![0u8; 0x40];
        b_code[0x17] = 0xAA;
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, vec![0xFFu8; 8]),
            ],
        );
        let result = compare_observations(&a, &b);
        let classes = vec![DivergenceClass::ElfHeader, DivergenceClass::Unclassified];
        let summary = summarize(&result, &classes);
        let first = render_summary_section(&summary, &a, &b);
        let second = render_summary_section(&summary, &a, &b);
        assert_eq!(first, second);
        assert!(first.contains("ElfHeader: 1 bytes"));
        assert!(first.contains("Unclassified: 8 bytes"));
    }

    #[test]
    fn render_unclassified_run_summarises_long_runs_with_head_tail() {
        let mut a_data = vec![0u8; 100];
        for (i, b) in a_data.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut b_data = vec![0u8; 100];
        for (i, b) in b_data.iter_mut().enumerate() {
            *b = (i + 0x80) as u8;
        }
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, a_data)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, b_data)],
        );
        let run = UnclassifiedRun {
            region_name: "data".to_string(),
            offset: 0,
            length: 100,
        };
        let line = render_unclassified_run(&run, &a, &b);
        assert!(line.contains("data@0x0+100"));
        assert!(line.contains(".."));
        assert!(line.contains("(100 bytes)"));
        assert!(
            line.contains("cellgov=0001020304050607") && line.contains("rpcs3=8081828384858687"),
            "head bytes missing: {line}"
        );
    }

    #[test]
    fn render_unclassified_run_names_runner_missing_region() {
        let with_region = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0u8; 4])],
        );
        let without_region = obs(ObservedOutcome::Completed, vec![]);
        let run = UnclassifiedRun {
            region_name: "data".to_string(),
            offset: 0,
            length: 4,
        };
        let only_cellgov = render_unclassified_run(&run, &with_region, &without_region);
        assert!(
            only_cellgov.contains("(region missing in rpcs3 observation)"),
            "got: {only_cellgov}"
        );
        let only_rpcs3 = render_unclassified_run(&run, &without_region, &with_region);
        assert!(
            only_rpcs3.contains("(region missing in cellgov observation)"),
            "got: {only_rpcs3}"
        );
        let neither = render_unclassified_run(&run, &without_region, &without_region);
        assert!(
            neither.contains("(region missing in both observations)"),
            "got: {neither}"
        );
    }

    #[test]
    fn region_slice_empty_length_returns_empty_vec() {
        let o = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0xAA; 4])],
        );
        assert_eq!(region_slice(&o, "r", 0, 0), Some(Vec::new()));
    }

    #[test]
    fn region_slice_offset_at_end_with_zero_length_is_some_empty() {
        let o = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0xAA; 4])],
        );
        assert_eq!(region_slice(&o, "r", 4, 0), Some(Vec::new()));
    }

    #[test]
    fn region_slice_offset_plus_length_at_end_is_inclusive_some() {
        let o = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0xAA, 0xBB, 0xCC, 0xDD])],
        );
        assert_eq!(region_slice(&o, "r", 2, 2), Some(vec![0xCC, 0xDD]));
    }

    #[test]
    fn region_slice_offset_plus_length_past_end_is_none() {
        let o = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0xAA; 4])],
        );
        assert_eq!(region_slice(&o, "r", 3, 2), None);
    }

    #[test]
    fn fixture_gen_produces_byte_deterministic_output_across_two_invocations() {
        let tmp = std::env::temp_dir().join(format!("cellgov_fixture_gen_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        struct CleanUp(std::path::PathBuf);
        impl Drop for CleanUp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _guard = CleanUp(tmp.clone());

        let a_obs = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 0x40])],
        );
        let mut b_data = vec![0u8; 0x40];
        b_data[0x17] = 0xAA;
        let b_obs = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
        );
        let result = compare_observations(&a_obs, &b_obs);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        let classes = classify_all(&result, &a_obs, &ctx);
        let summary = summarize(&result, &classes);

        write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
        let first = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
        write_compare_report(&tmp, &result, &summary, &a_obs, &b_obs).unwrap();
        let second = std::fs::read_to_string(tmp.join("compare_report.txt")).unwrap();
        assert_eq!(first, second, "two renders must produce identical bytes");
        assert!(first.contains("Convergence: Yes"));
        assert!(first.contains("Byte parity: 1 non-semantic"));
    }
}
