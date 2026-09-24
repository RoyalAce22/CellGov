//! `cellgov dev fixture-gen` -- regenerate the cross-runner triple
//! (`compare_report.txt`, `REPRODUCTION.md`,
//! `cross_runner_summary.json`) from two observations plus a title
//! manifest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_boot::classifier_context::{build_classifier_context, classify_all};
use cellgov_compare::{
    summarize, BootOverrides, ByteParity, Convergence, CrossRunnerSummary, Observation,
    ObservationCompareResult, ObservedOutcome, RunIdentity, UnclassifiedRun,
};

use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::FixtureGenArgs;
use super::self_load::{load_file, load_ppu_image_with_title};
use cellgov_boot::manifest::{CellKey, TitleManifest};

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

/// An absent half remains compatible with observations written before store versioning.
fn capture_identity_refusal(
    path: &str,
    captured: &RunIdentity,
    composed: &RunIdentity,
) -> Option<String> {
    if captured.firmware.is_some() && captured.firmware != composed.firmware {
        return Some(format!(
            "fixture-gen: {path} names a firmware identity that differs from the selected cell; \
             re-capture it with the same --fw selection used for this fixture"
        ));
    }
    if captured.game.is_some() && captured.game != composed.game {
        return Some(format!(
            "fixture-gen: {path} names a game identity that differs from the selected cell; \
             re-capture it with the same title and --game-ver selection used for this fixture"
        ));
    }
    None
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
    if let Some(refusal) =
        capture_identity_refusal(&cellgov_path, &cellgov.identity, &composition.identity)
    {
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
        oracle_gap_count(&vfs_root, &fixtures, &manifest.content_id, &cell)
            .map_err(|error| CommandError::failed(format!("fixture-gen: {error}")))?;

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

/// How many of the cell's unsupported syscalls the oracle's dispatch
/// table also leaves unbound, or `None` when the overlay or the cell's
/// anchor is absent.
///
/// # Errors
///
/// An overlay that is present but unreadable or malformed, naming the
/// file and, for a malformed one, the row; or an anchor that is present
/// but unreadable or not a boot summary, naming the file.
fn oracle_gap_count(
    vfs_root: &Path,
    fixtures: &Path,
    content_id: &str,
    cell: &CellKey,
) -> Result<Option<u64>, String> {
    let overlay = crate::oracle_gap::overlay_path(vfs_root);
    let gap = match std::fs::read_to_string(&overlay) {
        Ok(gap) => gap,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "read oracle-gap overlay {}: {error}",
                overlay.display()
            ))
        }
    };
    let ordinals = cellgov_lv2::archive::parse_overlay(&gap)
        .map_err(|error| format!("oracle-gap overlay {}: {error}", overlay.display()))?;
    let anchor = crate::paths::boot_anchor_path_in(fixtures, content_id, cell);
    // An anchor present but unreadable is refused, as the titles
    // generator refuses it, rather than recorded as "no count".
    let text = match std::fs::read_to_string(&anchor) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read boot anchor {}: {error}", anchor.display())),
    };
    let summary: cellgov_compare::BootSummary = serde_json::from_str(&text)
        .map_err(|error| format!("parse boot anchor {}: {error}", anchor.display()))?;
    Ok(Some(
        summary
            .unsupported_syscalls
            .keys()
            .filter(|ordinal| ordinals.contains(ordinal))
            .count() as u64,
    ))
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

#[cfg(test)]
#[path = "tests/fixture_gen_identity_tests.rs"]
mod identity_tests;
