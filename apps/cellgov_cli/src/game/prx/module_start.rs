//! `run_module_start`: drive a PRX's module_start to completion or
//! fault, with TTY capture and per-step ring buffers for diagnostics.

use std::collections::BTreeMap;
use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_mem::GuestMemory;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::process_address_space::PS3_PRIMARY_STACK_BASE;
use cellgov_time::Budget;

use crate::game::diag::{append_syscall_ring, fetch_raw_at, format_fault};
use crate::game::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use crate::game::step_loop::{RingCursor, PC_RING_SIZE, SYSCALL_RING_SIZE};

use super::tls::{install_kernel_context_opd, TLS_BASE};
use super::types::PrxLoadInfo;

/// Run a PRX module's module_start to completion or fault.
///
/// A decode-error fault at PC=0 with LR=0 at fault time is treated
/// as a clean return (the LR=0 sentinel set before entry).
pub(in crate::game) fn run_module_start(
    mut mem: GuestMemory,
    prx_info: &PrxLoadInfo,
    max_steps: usize,
    alloc_base: u32,
) -> GuestMemory {
    let ms = match prx_info.module_start {
        Some(opd) => opd,
        None => {
            println!(
                "module_start: {} has no module_start, skipping",
                prx_info.name
            );
            return mem;
        }
    };

    println!(
        "module_start: {} at pc=0x{:x} toc=0x{:x}",
        prx_info.name, ms.code, ms.toc,
    );

    let kctx_opd = install_kernel_context_opd(&mut mem);

    let mut ms_state = cellgov_ppu::state::PpuState::new();
    ms_state.pc = ms.code;
    ms_state.gpr[2] = ms.toc;
    // Offset below the game's stack_top so the two cannot collide
    // if a future caller runs them concurrently.
    ms_state.gpr[1] = PS3_PRIMARY_STACK_BASE + 0x8000;
    ms_state.gpr[11] = kctx_opd;
    ms_state.gpr[12] = kctx_opd;
    // PPC64 convention: r13 = TLS_area + 0x7030.
    ms_state.gpr[13] = TLS_BASE + 0x30 + 0x7000;
    // LR=0 sentinel: blr from module_start jumps to PC=0, where the
    // all-zero word fails to decode and the fault signals a return.
    ms_state.lr = 0;

    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = ms_state;
        unit
    });

    // Wall-clock display only, not ordering: never feeds
    // `sync_state_hash` or any scheduling decision.
    let t_start = Instant::now();
    let mut steps: usize = 0;
    let mut distinct_pcs = std::collections::BTreeSet::new();
    let mut hle_calls: BTreeMap<u32, usize> = BTreeMap::new();
    let mut lv2_calls: BTreeMap<u64, usize> = BTreeMap::new();
    let mut pc_hits: BTreeMap<u64, u64> = BTreeMap::new();
    let mut pc_ring: [u64; PC_RING_SIZE] = [0; PC_RING_SIZE];
    let mut pc_cursor = RingCursor::new(PC_RING_SIZE);
    // (nr, pc) per the shared append_syscall_ring schema.
    let mut sc_ring: [(u64, u64); SYSCALL_RING_SIZE] = [(0, 0); SYSCALL_RING_SIZE];
    let mut sc_cursor = RingCursor::new(SYSCALL_RING_SIZE);

    let outcome: String = loop {
        match rt.step() {
            Ok(step) => {
                steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    distinct_pcs.insert(pc);
                    *pc_hits.entry(pc).or_insert(0) += 1;
                    let idx = pc_cursor.record();
                    pc_ring[idx] = pc;
                }

                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *hle_calls.entry(idx).or_insert(0) += 1;
                    } else {
                        *lv2_calls.entry(args[0]).or_insert(0) += 1;
                    }
                    let sc_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let idx = sc_cursor.record();
                    sc_ring[idx] = (args[0], sc_pc);

                    if args[0] == cellgov_ps3_abi::syscall::TTY_WRITE {
                        handle_module_start_tty(args, rt.memory().as_bytes());
                    }
                }

                if steps.is_multiple_of(10_000) {
                    let hle_total: usize = hle_calls.values().sum();
                    let lv2_total: usize = lv2_calls.values().sum();
                    println!(
                        "  module_start [{:>6}] {} distinct PCs, {} HLE / {} LV2 calls",
                        steps,
                        distinct_pcs.len(),
                        hle_total,
                        lv2_total,
                    );
                }

                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    // Stepping further would run against state the
                    // computation never committed.
                    eprintln!("  module_start commit_step FAILED at step {steps}: {e:?}");
                    break format!("COMMIT_ERR {e:?} after {steps} steps");
                }

                if let Some(fault) = &step.result.fault {
                    let fault_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let guest_code = match fault {
                        cellgov_effects::FaultKind::Guest(c) => Some(*c),
                        _ => None,
                    };

                    // LR=0 check rejects corrupted call targets that
                    // happen to jump to PC=0 after an intermediate bl
                    // overwrote LR.
                    let lr_at_fault = step
                        .result
                        .local_diagnostics
                        .fault_regs
                        .as_ref()
                        .map(|r| r.lr)
                        .unwrap_or(u64::MAX);
                    if fault_pc == 0
                        && lr_at_fault == 0
                        && guest_code.is_some_and(cellgov_ppu::is_decode_error)
                    {
                        break format!("RETURNED after {} steps", steps);
                    }
                    let mut fault_text =
                        format_fault(&rt, &step.result, fault, steps, &pc_ring, &pc_cursor, &[]);
                    // format_fault renders PC ring only; append the
                    // syscall ring so module_start retains the same
                    // signal as a run-game fault.
                    append_syscall_ring(&mut fault_text, &sc_ring, &sc_cursor);
                    eprintln!("module_start {fault_text}");
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
                    let raw_str = match fetch_raw_at(&rt, fault_pc) {
                        Some(w) => format!("0x{w:08x}"),
                        None => "<unmapped>".to_string(),
                    };
                    break format!(
                        "FAULT {code_str} at pc=0x{fault_pc:x} (raw={raw_str}) after {steps} steps"
                    );
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                break format!("STALL after {} steps", steps);
            }
            Err(StepError::MaxStepsExceeded) => {
                break format!("MAX_STEPS after {} steps", steps);
            }
            Err(e) => {
                break format!("{e:?} after {steps} steps");
            }
        }
    };

    let elapsed = t_start.elapsed();
    println!(
        "module_start: {} -- {} steps, {} distinct PCs, {:.1?}",
        outcome,
        steps,
        distinct_pcs.len(),
        elapsed,
    );
    if !hle_calls.is_empty() {
        println!("  module_start HLE calls:");
        let mut sorted: Vec<_> = hle_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (idx, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  hle_{idx}");
        }
    }
    if !lv2_calls.is_empty() {
        println!("  module_start LV2 syscalls:");
        let mut sorted: Vec<_> = lv2_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (num, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  syscall {num}");
        }
    }

    if !pc_hits.is_empty() {
        println!("  module_start top PCs by hit count:");
        let mut sorted: Vec<_> = pc_hits.iter().collect();
        // Tie-break by PC so the ranking is independent of iteration order.
        sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
        for (pc, count) in sorted.iter().take(20) {
            let (raw, disasm) = match fetch_raw_at(&rt, **pc) {
                Some(w) => (
                    format!("0x{w:08x}"),
                    cellgov_ppu::decode::decode(w)
                        .ok()
                        .map(|insn| <&'static str>::from(&insn).to_string())
                        .unwrap_or_else(|| "<baddec>".into()),
                ),
                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
            };
            println!("    {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
        }
    }

    rt.into_memory()
}

fn handle_module_start_tty(args: &[u64; 9], mem: &[u8]) {
    match classify_tty_capture(args, mem) {
        TtyCaptureDecision::InBounds { bytes, .. } => {
            let preview = &bytes[..bytes.len().min(256)];
            let text = String::from_utf8_lossy(preview);
            print!("  module_start TTY: {text}");
        }
        TtyCaptureDecision::Oob { buf, len, mem_len } => {
            eprintln!(
                "  module_start TTY dropped: buf=0x{buf:x}+0x{len:x} exceeds guest memory (0x{mem_len:x})"
            );
        }
    }
}
