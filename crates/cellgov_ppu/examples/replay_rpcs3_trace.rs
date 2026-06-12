//! Replay every record from a CellGov PPU trace dump through the
//! differential harness.
//!
//! Usage: `replay_rpcs3_trace <path-to-dump> [<capture-id>] [--skip-context-dependent]`

#![allow(clippy::print_stdout, clippy::print_stderr)]

use cellgov_ppu::differential::rpcs3_capture::read_trace;
use cellgov_ppu::differential::{is_context_dependent, run_case, CaseOutcome};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path: Option<PathBuf> = None;
    let mut capture_id: Option<String> = None;
    let mut skip_context_dependent = false;
    for arg in std::env::args().skip(1) {
        if arg == "--skip-context-dependent" {
            skip_context_dependent = true;
        } else if path.is_none() {
            path = Some(PathBuf::from(arg));
        } else if capture_id.is_none() {
            capture_id = Some(arg);
        } else {
            eprintln!("usage: replay_rpcs3_trace <dump> [capture-id] [--skip-context-dependent]");
            std::process::exit(2);
        }
    }
    let path =
        path.ok_or("usage: replay_rpcs3_trace <dump> [capture-id] [--skip-context-dependent]")?;
    let capture_id_owned = capture_id.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_owned()
    });
    let capture_id: &'static str = Box::leak(capture_id_owned.into_boxed_str());

    let records = read_trace(&path)?;
    println!(
        "replaying {} records from {} (capture_id={}{})",
        records.len(),
        path.display(),
        capture_id,
        if skip_context_dependent {
            ", skip-context-dependent"
        } else {
            ""
        }
    );

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut shown = 0usize;
    for (i, record) in records.iter().enumerate() {
        if skip_context_dependent && is_context_dependent(record.raw_instruction) {
            skipped += 1;
            continue;
        }
        let label = format!("rec_{i}_pc_0x{:08x}", record.pc);
        let case = record.to_instruction_case(label, capture_id);
        match run_case(&case) {
            CaseOutcome::Pass => passed += 1,
            other => {
                failed += 1;
                if shown < 10 {
                    println!(
                        "  FAIL [{i}] pc=0x{:08x} raw=0x{:08x}: {other:?}",
                        record.pc, record.raw_instruction
                    );
                    if is_context_dependent(record.raw_instruction) {
                        println!(
                            "    diag: rtime pre=0x{:016x} post=0x{:016x} (delta=0x{:016x})",
                            record.pre_reservation_rtime,
                            record.post_reservation_rtime,
                            record
                                .post_reservation_rtime
                                .wrapping_sub(record.pre_reservation_rtime),
                        );
                    }
                    shown += 1;
                }
            }
        }
    }

    println!(
        "summary: {passed} pass / {failed} fail / {skipped} context-dependent (of {})",
        records.len()
    );
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
