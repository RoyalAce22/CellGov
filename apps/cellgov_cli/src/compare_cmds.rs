//! `compare-observations`, `diverge`, and `zoom` subcommand handlers.
//!
//! These three commands share the "load two files and report a
//! divergence" shape, so they live together in this module.

use crate::{die, load_file_or_die};

/// `cellgov_cli zoom <a.zoom.state> <b.zoom.state> <step>` -- loads
/// two zoom-trace byte streams, runs `cellgov_compare::zoom_lookup`
/// at the named step, and prints a per-field register diff.
///
/// Exit codes: 0 on identical-state hash collision, 1 on a real diff,
/// 2 when the requested step is missing from one or both windows.
pub fn run_zoom(a_path: &str, b_path: &str, step: u64) {
    use cellgov_compare::{zoom_lookup, ZoomLookup};
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    match zoom_lookup(&a_bytes, &b_bytes, step) {
        ZoomLookup::Found {
            step,
            a_pc,
            b_pc,
            diffs,
        } => {
            if diffs.is_empty() {
                println!("HASH_COLLISION step={step} pc=0x{a_pc:x}  full snapshots are byte-equal; resume scan from step {next}", next = step + 1);
            } else {
                println!(
                    "ZOOM step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  {} field(s) differ:",
                    diffs.len()
                );
                for d in &diffs {
                    println!("  {:<5}  a=0x{:016x}  b=0x{:016x}", d.field, d.a, d.b);
                }
                std::process::exit(1);
            }
        }
        ZoomLookup::MissingStep {
            step,
            a_missing,
            b_missing,
        } => {
            // Positive-sense locals so the format string reads
            // cleanly; the `!` lives in exactly one place.
            let a_has_step = !a_missing;
            let b_has_step = !b_missing;
            println!(
                "MISSING_STEP step={step}  a_has_step={a_has_step}  b_has_step={b_has_step}  (zoom window did not cover this step on at least one side)"
            );
            std::process::exit(2);
        }
    }
}

/// `cellgov_cli diverge <a.state> <b.state>` -- streaming scan of two
/// per-step state-trace files. Prints IDENTICAL / DIVERGE / LENGTH_DIFFERS
/// and exits non-zero on any non-identical outcome.
pub fn run_diverge(a_path: &str, b_path: &str) {
    use cellgov_compare::{diverge, DivergeField, DivergeReport};
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    match diverge(&a_bytes, &b_bytes) {
        DivergeReport::Identical { count } => {
            println!("IDENTICAL  {count} PpuStateHash records matched");
            // Zero-record "identical" is almost always a malformed
            // or truncated trace, not two legitimately empty runs.
            // Warn so the operator does not take a trivially-vacuous
            // match as a real signal.
            if count == 0 {
                eprintln!(
                    "WARN: zero PpuStateHash records matched; trace files may be empty or truncated"
                );
            }
        }
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            let field_str = match field {
                DivergeField::Pc => "pc",
                DivergeField::Hash => "hash",
            };
            println!(
                "DIVERGE step={step} field={field_str}  a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  a_hash=0x{a_hash:x} b_hash=0x{b_hash:x}"
            );
            std::process::exit(1);
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            println!(
                "LENGTH_DIFFERS  common={common_count}  a={a_count}  b={b_count}  ({a_path} vs {b_path})"
            );
            std::process::exit(1);
        }
    }
}

/// `cellgov_cli compare-observations <a.json> <b.json>` -- diffs two
/// JSON-encoded `Observation` files (region-by-region byte equality
/// plus outcome match). Stops at the first divergence with a typed
/// label.
pub fn run_compare_observations(a_path: &str, b_path: &str) {
    let a_bytes = load_file_or_die(a_path);
    let b_bytes = load_file_or_die(b_path);
    let a: cellgov_compare::Observation =
        serde_json::from_slice(&a_bytes).unwrap_or_else(|e| die(&format!("parse {a_path}: {e}")));
    let b: cellgov_compare::Observation =
        serde_json::from_slice(&b_bytes).unwrap_or_else(|e| die(&format!("parse {b_path}: {e}")));

    if a.outcome != b.outcome {
        println!("DIVERGE outcome: {:?} vs {:?}", a.outcome, b.outcome);
        std::process::exit(1);
    }
    if a.memory_regions.len() != b.memory_regions.len() {
        println!(
            "DIVERGE region count: {} vs {}",
            a.memory_regions.len(),
            b.memory_regions.len()
        );
        std::process::exit(1);
    }
    for (ra, rb) in a.memory_regions.iter().zip(b.memory_regions.iter()) {
        if ra.name != rb.name || ra.addr != rb.addr {
            println!(
                "DIVERGE region identity: {}@0x{:x} vs {}@0x{:x}",
                ra.name, ra.addr, rb.name, rb.addr
            );
            std::process::exit(1);
        }
        // Length check FIRST: if one side is a prefix of the other,
        // `zip(...).position(x != y)` returns None and unwrap_or(0)
        // would report "first byte differs at 0x0" with identical
        // bytes (xx vs xx). The operator would see contradictory
        // output and assume a tool bug when it is actually a length
        // mismatch.
        if ra.data.len() != rb.data.len() {
            println!(
                "DIVERGE region {}: length {} vs {} bytes",
                ra.name,
                ra.data.len(),
                rb.data.len()
            );
            std::process::exit(1);
        }
        if ra.data != rb.data {
            // Lengths are equal here, so `position` is guaranteed to
            // find the first differing byte when the slices differ.
            let first_diff = ra
                .data
                .iter()
                .zip(rb.data.iter())
                .position(|(x, y)| x != y)
                .expect("equal-length slices that differ must have a first diff");
            println!(
                "DIVERGE region {}: first byte differs at offset 0x{:x} (guest 0x{:x}) -- {:02x} vs {:02x}",
                ra.name,
                first_diff,
                ra.addr + first_diff as u64,
                ra.data[first_diff],
                rb.data[first_diff],
            );
            std::process::exit(1);
        }
    }
    let total_bytes: usize = a.memory_regions.iter().map(|r| r.data.len()).sum();
    println!(
        "MATCH outcome={:?}, {} regions ({} bytes) identical, steps {:?} vs {:?}",
        a.outcome,
        a.memory_regions.len(),
        total_bytes,
        a.metadata.steps,
        b.metadata.steps,
    );
    // Zero regions + zero bytes is almost always an upstream
    // observation-capture bug rather than a deliberate empty run.
    // Two empty observations trivially compare equal, which would
    // quietly hide the real problem.
    if a.memory_regions.is_empty() {
        eprintln!(
            "WARN: both observations carry zero memory regions; comparison is trivially vacuous"
        );
    }
    // Step-count mismatch semantics:
    //
    // Different runners (e.g. CellGov vs RPCS3) count step units
    // differently -- CellGov counts PPU instruction retirements,
    // RPCS3 counts something else -- so a step-count disagreement
    // at byte-equal state is expected and does not invalidate the
    // MATCH verdict. Print a stderr NOTE so an operator who cares
    // can see it, but keep exit 0.
    //
    // Same runner is different: two runs of the same deterministic
    // runner that reach byte-equal state in different step counts
    // is a genuine divergence (non-determinism, ordering drift, a
    // scheduler bug). In that case promote to a DIVERGE verdict
    // and exit 1 so CI catches it.
    if let (Some(sa), Some(sb)) = (a.metadata.steps, b.metadata.steps) {
        if sa != sb {
            if a.metadata.runner == b.metadata.runner {
                println!(
                    "DIVERGE step count: {sa} vs {sb} within runner '{}' (byte-equal state reached via different work -- a determinism failure)",
                    a.metadata.runner
                );
                std::process::exit(1);
            }
            eprintln!(
                "NOTE: step counts differ ({sa} vs {sb}); cross-runner comparison between '{}' and '{}' does not require matching step counts",
                a.metadata.runner, b.metadata.runner
            );
        }
    }
}
