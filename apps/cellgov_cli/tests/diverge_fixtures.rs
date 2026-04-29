//! Integration fixtures for the diverge scanner. Wires cellgov_ppu
//! state-hash traces through cellgov_compare scanning.

use cellgov_compare::{diverge, DivergeField, DivergeReport};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

/// Encode N back-to-back `addi` instructions starting at address 0.
/// rT cycles 3,4,5 so adjacent state hashes differ.
fn linear_addi_program(n: usize) -> GuestMemory {
    let mut mem = GuestMemory::new(4096);
    for i in 0..n {
        let rt = 3 + (i % 3) as u32;
        let raw: u32 = (14 << 26) | (rt << 21) | ((i as u32) + 1);
        let range = ByteRange::new(GuestAddr::new((i * 4) as u64), 4).unwrap();
        mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
    }
    mem
}

/// Run a PPU unit for `n` instructions and serialize the retired
/// `(pc, hash)` pairs as `PpuStateHash` records -- same byte layout
/// the runtime emits.
fn ppu_trace_bytes(mem: &GuestMemory, n: usize) -> Vec<u8> {
    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(mem).with_trace_per_step(true);
    let _ = ppu.run_until_yield(Budget::new(n as u64), &ctx, &mut Vec::new());
    let pairs = ppu.drain_retired_state_hashes();
    let mut writer = TraceWriter::new();
    for (i, (pc, hash)) in pairs.into_iter().enumerate() {
        writer.record(&TraceRecord::PpuStateHash {
            step: i as u64,
            pc,
            hash: StateHash::new(hash),
        });
    }
    writer.take_bytes()
}

#[test]
fn ppu_run_twice_with_per_step_trace_is_byte_identical() {
    let mem = linear_addi_program(20);
    let a = ppu_trace_bytes(&mem, 20);
    let b = ppu_trace_bytes(&mem, 20);

    assert_eq!(
        a, b,
        "two runs of the same scenario with per-step tracing must produce byte-identical state traces"
    );
    assert_eq!(diverge(&a, &b), DivergeReport::Identical { count: 20 });
}

#[test]
fn seeded_gpr_mutation_locates_at_expected_step() {
    let n = 20;
    let mem = linear_addi_program(n);
    let a = ppu_trace_bytes(&mem, n);

    // Swap rT 5 -> 6 at step 5 so post-instruction state differs
    // without changing PC flow.
    let mut mutated = linear_addi_program(n);
    let mutated_step: u64 = 5;
    let pc_to_mutate = mutated_step * 4;
    let new_rt: u32 = 6;
    let raw: u32 = (14 << 26) | (new_rt << 21) | (mutated_step as u32 + 1);
    let range = ByteRange::new(GuestAddr::new(pc_to_mutate), 4).unwrap();
    mutated.apply_commit(range, &raw.to_be_bytes()).unwrap();

    let b = ppu_trace_bytes(&mutated, n);

    match diverge(&a, &b) {
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            field,
            ..
        } => {
            assert_eq!(
                step, mutated_step,
                "divergence should localize to step {mutated_step}"
            );
            assert_eq!(a_pc, pc_to_mutate, "a_pc should be the mutated PC");
            assert_eq!(
                b_pc, pc_to_mutate,
                "b_pc should match a_pc (no control-flow change)"
            );
            assert_eq!(
                field,
                DivergeField::Hash,
                "same PC, different post-instruction state -> hash field"
            );
        }
        other => panic!("expected Differs at step {mutated_step}, got {other:?}"),
    }
}

#[test]
fn cli_diverge_subcommand_reports_identical_on_match() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = std::env::temp_dir().join("cellgov_9e2_match");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mem = linear_addi_program(8);
    let bytes = ppu_trace_bytes(&mem, 8);
    let a = dir.join("a.state");
    let b = dir.join("b.state");
    std::fs::write(&a, &bytes).unwrap();
    std::fs::write(&b, &bytes).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov_cli"));
    let out = Command::new(bin)
        .args(["diverge", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "diverge exit non-zero: {stdout}");
    assert!(
        stdout.contains("IDENTICAL"),
        "expected IDENTICAL in output, got: {stdout}"
    );
    assert!(
        stdout.contains("8 PpuStateHash records"),
        "expected matched-count, got: {stdout}"
    );
}

#[test]
fn cli_diverge_subcommand_reports_diverge_on_mismatch() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = std::env::temp_dir().join("cellgov_9e2_diverge");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mem = linear_addi_program(8);
    let a_bytes = ppu_trace_bytes(&mem, 8);
    let mut mutated = linear_addi_program(8);
    let raw: u32 = (14 << 26) | (6 << 21) | 4;
    let range = ByteRange::new(GuestAddr::new(12), 4).unwrap();
    mutated.apply_commit(range, &raw.to_be_bytes()).unwrap();
    let b_bytes = ppu_trace_bytes(&mutated, 8);

    let a = dir.join("a.state");
    let b = dir.join("b.state");
    std::fs::write(&a, &a_bytes).unwrap();
    std::fs::write(&b, &b_bytes).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov_cli"));
    let out = Command::new(bin)
        .args(["diverge", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "diverge should exit non-zero on divergence: {stdout}"
    );
    assert!(
        stdout.contains("DIVERGE step=3"),
        "expected DIVERGE step=3, got: {stdout}"
    );
    assert!(
        stdout.contains("field=hash"),
        "expected field=hash, got: {stdout}"
    );
}

/// Produce a zoom-trace byte buffer from a PPU run with a configured
/// full-state window, serialized as `PpuStateFull` records.
fn ppu_zoom_bytes(mem: &GuestMemory, n: usize, window: (u64, u64)) -> Vec<u8> {
    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    ppu.set_full_state_window(Some(window));
    let ctx = ExecutionContext::new(mem);
    let _ = ppu.run_until_yield(Budget::new(n as u64), &ctx, &mut Vec::new());
    let pairs = ppu.drain_retired_state_full();
    let mut writer = TraceWriter::new();
    for (i, (pc, gpr, lr, ctr, xer, cr)) in pairs.into_iter().enumerate() {
        writer.record(&TraceRecord::PpuStateFull {
            step: window.0 + i as u64,
            pc,
            gpr,
            lr,
            ctr,
            xer,
            cr,
        });
    }
    writer.take_bytes()
}

#[test]
fn cli_zoom_subcommand_names_mutated_register_field() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = std::env::temp_dir().join("cellgov_9g3_zoom");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();

    let n = 20usize;
    let window = (4u64, 6u64);
    let mem_a = linear_addi_program(n);
    let a_zoom = ppu_zoom_bytes(&mem_a, n, window);

    let mut mem_b = linear_addi_program(n);
    // Step 5: swap rt from 5 to 6, keeping imm unchanged.
    let raw: u32 = (14 << 26) | (6 << 21) | 6;
    let range = ByteRange::new(GuestAddr::new(20), 4).unwrap();
    mem_b.apply_commit(range, &raw.to_be_bytes()).unwrap();
    let b_zoom = ppu_zoom_bytes(&mem_b, n, window);

    let a = dir.join("a.zoom.state");
    let b = dir.join("b.zoom.state");
    std::fs::write(&a, &a_zoom).unwrap();
    std::fs::write(&b, &b_zoom).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov_cli"));
    let out = Command::new(bin)
        .args(["zoom", a.to_str().unwrap(), b.to_str().unwrap(), "5"])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "zoom should exit non-zero on real diff: {stdout}"
    );
    assert!(
        stdout.contains("ZOOM step=5"),
        "expected ZOOM header line, got: {stdout}"
    );
    assert!(
        stdout.contains("gpr5"),
        "expected gpr5 named in diff, got: {stdout}"
    );
    assert!(
        stdout.contains("gpr6"),
        "expected gpr6 named in diff, got: {stdout}"
    );
}

#[test]
fn cli_zoom_reports_hash_collision_when_full_states_match() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = std::env::temp_dir().join("cellgov_9g4_collision");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mem = linear_addi_program(20);
    let z = ppu_zoom_bytes(&mem, 20, (4, 6));
    let a = dir.join("a.zoom.state");
    let b = dir.join("b.zoom.state");
    std::fs::write(&a, &z).unwrap();
    std::fs::write(&b, &z).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov_cli"));
    let out = Command::new(bin)
        .args(["zoom", a.to_str().unwrap(), b.to_str().unwrap(), "5"])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "collision case should exit 0 so outer scan resumes: {stdout}"
    );
    assert!(
        stdout.contains("HASH_COLLISION"),
        "expected HASH_COLLISION line, got: {stdout}"
    );
    assert!(
        stdout.contains("resume scan from step 6"),
        "expected next-step resume hint, got: {stdout}"
    );
}

#[test]
fn cli_zoom_reports_missing_step_when_window_excluded_it() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = std::env::temp_dir().join("cellgov_9g_missing");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mem = linear_addi_program(20);
    let z = ppu_zoom_bytes(&mem, 20, (0, 2));
    let a = dir.join("a.zoom.state");
    let b = dir.join("b.zoom.state");
    std::fs::write(&a, &z).unwrap();
    std::fs::write(&b, &z).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_cellgov_cli"));
    let out = Command::new(bin)
        .args(["zoom", a.to_str().unwrap(), b.to_str().unwrap(), "10"])
        .output()
        .expect("cli runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stdout.contains("MISSING_STEP"),
        "expected MISSING_STEP line, got: {stdout}"
    );
}

#[test]
fn truncated_b_reports_length_mismatch_at_truncation_point() {
    // Side B halted mid-run; the common prefix matches. Distinguishes
    // length-mismatch from content divergence.
    let mem = linear_addi_program(20);
    let a = ppu_trace_bytes(&mem, 20);
    let b = ppu_trace_bytes(&mem, 7);

    match diverge(&a, &b) {
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            assert_eq!(common_count, 7);
            assert_eq!(a_count, 20);
            assert_eq!(b_count, 7);
        }
        other => panic!("expected LengthDiffers, got {other:?}"),
    }
}
