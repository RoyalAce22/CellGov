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
            println!(
                "MISSING_STEP step={step}  a_has_step={}  b_has_step={}  (zoom window did not cover this step on at least one side)",
                !a_missing,
                !b_missing
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
        if ra.data != rb.data {
            let first_diff = ra
                .data
                .iter()
                .zip(rb.data.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(0);
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
    println!(
        "MATCH outcome={:?}, {} regions ({} bytes) identical, steps {:?} vs {:?}",
        a.outcome,
        a.memory_regions.len(),
        a.memory_regions.iter().map(|r| r.data.len()).sum::<usize>(),
        a.metadata.steps,
        b.metadata.steps,
    );
}
