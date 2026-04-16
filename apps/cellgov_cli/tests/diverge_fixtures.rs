//! Integration fixtures for the diverge scanner.
//!
//! These wire together cellgov_ppu (which produces per-step state-hash
//! traces) and cellgov_compare (which scans them) -- two crates with
//! no dependency between them.
//!
//! Self-determinism: a tiny PPU program executed twice with per-step
//! tracing on must produce byte-identical state traces, and the
//! diverge scanner must report `Identical`.
//!
//! Seeded divergence: the same PPU program with a one-byte mutation
//! applied to a known instruction must diverge at the expected step
//! index, with the diverge scanner naming either the mutated PC
//! (control flow change) or the post-instruction hash (data change
//! at the same PC).

use cellgov_compare::{diverge, DivergeField, DivergeReport};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

/// Encode a sequence of N back-to-back `addi` instructions starting at
/// guest address 0. Each instruction sets a different GPR so adjacent
/// state hashes differ. Returns the populated guest memory.
fn linear_addi_program(n: usize) -> GuestMemory {
    let mut mem = GuestMemory::new(4096);
    for i in 0..n {
        // addi rT, r0, (i+1)  -- rT cycles 3,4,5,3,4,5,... so successive
        // adjacent instructions touch different registers.
        let rt = 3 + (i % 3) as u32;
        let raw: u32 = (14 << 26) | (rt << 21) | ((i as u32) + 1);
        let range = ByteRange::new(GuestAddr::new((i * 4) as u64), 4).unwrap();
        mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
    }
    mem
}

/// Run a PPU unit through `n` instructions with per-step trace on,
/// drain the resulting `(pc, hash)` pairs, and serialize them into a
/// trace-byte buffer as `PpuStateHash` records (step indices monotonic
/// from 0). This is the same byte layout the real runtime would emit.
fn ppu_trace_bytes(mem: &GuestMemory, n: usize) -> Vec<u8> {
    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    ppu.set_per_step_trace(true);
    let ctx = ExecutionContext::new(mem);
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
    // Same scenario, two runs, byte-identical state trace. Plus
    // the diverge scanner reports Identical with the expected count.
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
    // Produce side A normally, then produce side B from an
    // execution where one register was perturbed mid-stream by
    // mutating one instruction's destination register. Diverge must
    // localize the divergence to the step at which the mutated
    // instruction retired.
    let n = 20;
    let mem = linear_addi_program(n);
    let a = ppu_trace_bytes(&mem, n);

    // Mutate the instruction at PC 0x14 (5th instruction, step index
    // 5): change addi r4, r0, 6 -> addi r5, r0, 6 by replacing the
    // instruction word in memory before the second run.
    let mut mutated = linear_addi_program(n);
    let mutated_step: u64 = 5;
    let pc_to_mutate = mutated_step * 4;
    // Original word at offset 20 has rt = 3 + (5 % 3) = 5. Switch rt
    // to 6 to alter post-instruction state without changing PC flow.
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
    // Drive the actual cellgov_cli binary against two byte-identical
    // state-trace files, asserting the human-readable IDENTICAL line.
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
    let raw: u32 = (14 << 26) | (6 << 21) | 4; // step 3: mutated rt
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

/// Helper for 9G zoom-in tests: produce a zoom-trace byte buffer
/// from a PPU run with a configured window. Returns the bytes and
/// the final retired-instruction count for index assertions.
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
    // Produce two zoom traces around step 5 of the same
    // 20-instruction program where one side mutated the destination
    // register at step 5. CLI zoom subcommand must name gpr5 (or
    // whichever rt the mutation switched to).
    let dir = std::env::temp_dir().join("cellgov_9g3_zoom");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();

    let n = 20usize;
    let window = (4u64, 6u64); // [N-1, N+1] around step 5
    let mem_a = linear_addi_program(n);
    let a_zoom = ppu_zoom_bytes(&mem_a, n, window);

    let mut mem_b = linear_addi_program(n);
    // Mutate step 5 (PC 0x14, was rt=5 imm=6) to rt=6 imm=6.
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
    // Side A's r5 was set to 6 (original). Side B's r5 stayed 0
    // (mutation moved write to r6). So gpr5 differs.
    assert!(
        stdout.contains("gpr5"),
        "expected gpr5 named in diff, got: {stdout}"
    );
    // Side B's r6 was set to 6 (mutated write). Side A's r6 stayed 0
    // (original sequence next set it later). So gpr6 also differs.
    assert!(
        stdout.contains("gpr6"),
        "expected gpr6 named in diff, got: {stdout}"
    );
}

#[test]
fn cli_zoom_reports_hash_collision_when_full_states_match() {
    use std::path::PathBuf;
    use std::process::Command;
    // Identical state at step 5 -> zoom says HASH_COLLISION
    // and exits 0 so the outer scanner can resume.
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
    // Window covers [0, 2]; query step 10 -> MissingStep, exit 2.
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
    // Edge case: side B was halted mid-run. The common prefix matched
    // up to that point. Diverge should distinguish this from a
    // content divergence.
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
