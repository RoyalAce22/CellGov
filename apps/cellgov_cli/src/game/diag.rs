//! Diagnostic formatting and printing for `run-game`.
//!
//! Split out of `game.rs` to keep the core boot driver manageable.
//! Every function here is pure formatting: it reads state and produces
//! a String or stdout output, and does not mutate guest state.

use crate::game::{PC_RING_SIZE, SYSCALL_RING_SIZE};
use cellgov_core::Runtime;

/// Fetch a 32-bit big-endian instruction word from guest memory at `pc`.
pub(super) fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let m = rt.memory().as_bytes();
    let a = pc as usize;
    if a + 4 <= m.len() {
        Some(u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]]))
    } else {
        None
    }
}

/// Captured TTY write for the diagnostic artifact.
pub(super) struct TtyCapture {
    pub(super) fd: u32,
    pub(super) raw_bytes: Vec<u8>,
    pub(super) call_pc: u64,
}

/// Captured sys_process_exit info.
pub(super) struct ProcessExitInfo {
    pub(super) code: u32,
    pub(super) call_pc: u64,
}

pub(super) fn print_trace_line(
    rt: &Runtime,
    result: &cellgov_exec::ExecutionStepResult,
    steps: usize,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    if let Some(pc) = result.local_diagnostics.pc {
        let raw = fetch_raw_at(rt, pc).unwrap_or(0);
        println!(
            "[{steps:>4}] PC=0x{pc:08x}  raw=0x{raw:08x}  yr={:?}",
            result.yield_reason
        );
    }
    if let Some(args) = &result.syscall_args {
        if args[0] >= 0x10000 {
            let idx = (args[0] - 0x10000) as u32;
            let name = hle_bindings
                .get(idx as usize)
                .map(|b| {
                    let func = cellgov_ppu::nid_db::lookup(b.nid)
                        .map(|(_, f)| f)
                        .unwrap_or("?");
                    format!("{}::{}", b.module, func)
                })
                .unwrap_or_else(|| format!("hle_{idx}"));
            println!("       -> HLE #{idx}: {name}");
        } else if args[0] == 403 {
            let buf = args[2] as usize;
            let len = args[3] as usize;
            let m = rt.memory().as_bytes();
            if buf + len <= m.len() {
                let text = String::from_utf8_lossy(&m[buf..buf + len]);
                print!("       -> tty: {text}");
                if !text.ends_with('\n') {
                    println!();
                }
            } else {
                println!("       -> LV2 tty_write (oob)");
            }
        } else {
            println!("       -> LV2 syscall {}", args[0]);
        }
    }
}

pub(super) fn format_fault(
    rt: &Runtime,
    result: &cellgov_exec::ExecutionStepResult,
    fault: &cellgov_effects::FaultKind,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_ring_pos: usize,
) -> String {
    let pc = result.local_diagnostics.pc;
    let pc_str = pc
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    use cellgov_ppu::{
        FAULT_DEBUG_BREAK, FAULT_DECODE_ERROR, FAULT_INVALID_ADDRESS, FAULT_PC_OUT_OF_RANGE,
        FAULT_UNSUPPORTED_SYSCALL,
    };
    let detail = match fault {
        cellgov_effects::FaultKind::Guest(code) => {
            let fault_type = code & 0xFFFF_0000;
            match fault_type {
                FAULT_PC_OUT_OF_RANGE => format!("PC_OUT_OF_RANGE at PC={pc_str}"),
                FAULT_DECODE_ERROR => {
                    let raw_str = pc
                        .and_then(|a| fetch_raw_at(rt, a))
                        .map(|w| format!("0x{w:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    format!("DECODE_ERROR at PC={pc_str} (raw={raw_str})")
                }
                FAULT_INVALID_ADDRESS => {
                    let ea_str = result
                        .local_diagnostics
                        .faulting_ea
                        .map(|a| format!("0x{a:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    format!("INVALID_ADDRESS at PC={pc_str} (ea={ea_str})")
                }
                FAULT_UNSUPPORTED_SYSCALL => {
                    let nr = code & 0x0000_FFFF;
                    format!("UNSUPPORTED_SYSCALL (nr={nr}) at PC={pc_str}")
                }
                FAULT_DEBUG_BREAK => {
                    let mut s = format!("DEBUG_BREAK at PC={pc_str}");
                    // Dump memory at each GPR that looks like a guest pointer.
                    if let Some(regs) = &result.local_diagnostics.fault_regs {
                        let mem = rt.memory().as_bytes();
                        for (i, &val) in regs.gprs.iter().enumerate() {
                            let addr = val as usize;
                            if addr < mem.len() && addr > 0x1000 {
                                let end = (addr + 64).min(mem.len());
                                s.push_str(&format!("\n  [r{i}=0x{val:08x}]: "));
                                let slice = &mem[addr..end];
                                // Show printable ASCII if it looks like a string.
                                let printable = slice
                                    .iter()
                                    .take_while(|&&b| (0x20..0x7f).contains(&b))
                                    .count();
                                if printable >= 4 {
                                    let text: String = slice
                                        .iter()
                                        .take_while(|&&b| (0x20..0x7f).contains(&b))
                                        .map(|&b| b as char)
                                        .collect();
                                    s.push_str(&format!("{text:?}"));
                                } else {
                                    for b in &slice[..16.min(slice.len())] {
                                        s.push_str(&format!("{b:02x} "));
                                    }
                                }
                            }
                        }
                    }
                    s
                }
                _ => format!("Guest(0x{code:08x}) at PC={pc_str}"),
            }
        }
        _ => format!("Validation at PC={pc_str}"),
    };
    let mut out = format!("FAULT at step {steps}: {detail}");

    // Register dump if available.
    if let Some(regs) = &result.local_diagnostics.fault_regs {
        out.push_str("\n  registers:");
        for (i, &val) in regs.gprs.iter().enumerate() {
            if i % 4 == 0 {
                out.push_str("\n    ");
            }
            out.push_str(&format!("r{i:<2}=0x{val:016x}  "));
        }
        out.push_str(&format!(
            "\n    LR=0x{:016x}  CTR=0x{:016x}  CR=0x{:08x}",
            regs.lr, regs.ctr, regs.cr
        ));
    }

    // Mini-trace: last N PCs from the ring buffer, each with the raw
    // word and decoded mnemonic. For memory-access faults, walking
    // back through the mnemonics identifies the instruction that
    // computed the bad effective address.
    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            let raw = fetch_raw_at(rt, pc)
                .map(|w| format!("0x{w:08x}"))
                .unwrap_or_else(|| "?".to_string());
            let name = fetch_raw_at(rt, pc)
                .and_then(|w| cellgov_ppu::decode::decode(w).ok())
                .map(|insn| insn.variant_name().to_string())
                .unwrap_or_else(|| "?".into());
            out.push_str(&format!("\n    0x{pc:08x}  raw={raw}  {name}"));
        }
    }

    out
}

/// Format the diagnostic artifact for a guest-initiated sys_process_exit.
///
/// Includes: exit code, call-site PC, last 16 PCs, and hex dump + decoded
/// string of the most recent TTY write (the error message).
#[allow(clippy::too_many_arguments)]
pub(super) fn format_process_exit(
    exit: &ProcessExitInfo,
    last_tty: Option<&TtyCapture>,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_ring_pos: usize,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_ring_pos: usize,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) -> String {
    let mut out = format!(
        "PROCESS_EXIT(code={}) at step {} (PC=0x{:08x})",
        exit.code, steps, exit.call_pc
    );

    // Last TTY write (the error message).
    if let Some(tty) = last_tty {
        out.push_str(&format!(
            "\n  last tty write (fd={}, {} bytes, PC=0x{:08x}):",
            tty.fd,
            tty.raw_bytes.len(),
            tty.call_pc,
        ));
        // Hex dump.
        for chunk in tty.raw_bytes.chunks(16) {
            out.push_str("\n    ");
            for (i, b) in chunk.iter().enumerate() {
                if i == 8 {
                    out.push(' ');
                }
                out.push_str(&format!("{b:02x} "));
            }
        }
        // Decoded string.
        let text = String::from_utf8_lossy(&tty.raw_bytes);
        out.push_str(&format!("\n  decoded: {:?}", text.trim_end()));
    }

    // Mini-trace: last N PCs.
    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            out.push_str(&format!("\n    0x{pc:08x}"));
        }
    }

    // Last N syscalls.
    let sc_filled = syscall_ring_pos.min(SYSCALL_RING_SIZE);
    if sc_filled > 0 {
        out.push_str(&format!("\n  last {sc_filled} syscalls:"));
        let start = syscall_ring_pos.saturating_sub(SYSCALL_RING_SIZE);
        for i in start..syscall_ring_pos {
            let (nr, pc) = syscall_ring[i % SYSCALL_RING_SIZE];
            if nr >= 0x10000 {
                let idx = (nr - 0x10000) as u32;
                let name = hle_bindings
                    .get(idx as usize)
                    .and_then(|b| cellgov_ppu::nid_db::lookup(b.nid).map(|(_, f)| f.to_string()))
                    .unwrap_or_else(|| format!("hle_{idx}"));
                out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
            } else {
                out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
            }
        }
    }

    out
}

/// Format the MAX_STEPS diagnostic: step count plus the last 16 PCs and
/// last 32 syscalls. The hot loop body is whichever PCs dominate the
/// top-PC histogram (printed separately by `print_top_pcs`); this ring
/// shows the most recent branch flow and any syscalls made just before
/// the cap, which are the candidate places the stall originated.
pub(super) fn format_max_steps(
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_ring_pos: usize,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_ring_pos: usize,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) -> String {
    let mut out = format!("MAX_STEPS after {} steps", steps);

    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            out.push_str(&format!("\n    0x{pc:08x}"));
        }
    }

    let sc_filled = syscall_ring_pos.min(SYSCALL_RING_SIZE);
    if sc_filled > 0 {
        out.push_str(&format!("\n  last {sc_filled} syscalls:"));
        let start = syscall_ring_pos.saturating_sub(SYSCALL_RING_SIZE);
        for i in start..syscall_ring_pos {
            let (nr, pc) = syscall_ring[i % SYSCALL_RING_SIZE];
            if nr >= 0x10000 {
                let idx = (nr - 0x10000) as u32;
                let name = hle_bindings
                    .get(idx as usize)
                    .and_then(|b| cellgov_ppu::nid_db::lookup(b.nid).map(|(_, f)| f.to_string()))
                    .unwrap_or_else(|| format!("hle_{idx}"));
                out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
            } else {
                out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
            }
        }
    }

    out
}

pub(super) fn print_hle_summary(
    hle_calls: &std::collections::BTreeMap<u32, usize>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    let called_count = hle_calls.len();
    let total_count = hle_bindings.len();
    let uncalled_count = total_count - called_count.min(total_count);
    println!("hle_imports: {total_count} bound, {called_count} called, {uncalled_count} uncalled");

    if !hle_calls.is_empty() {
        println!("  called:");
        for (idx, count) in hle_calls {
            let (name, class) = hle_bindings
                .get(*idx as usize)
                .map(|b| {
                    let func = cellgov_ppu::nid_db::lookup(b.nid)
                        .map(|(_, f)| f)
                        .unwrap_or("?");
                    (
                        format!("{}::{}", b.module, func),
                        cellgov_ppu::nid_db::stub_classification(b.nid),
                    )
                })
                .unwrap_or_else(|| (format!("hle_{idx}"), "?"));
            println!("    {name}: {count}x [{class}]");
        }
    }

    // Show uncalled imports grouped by classification.
    let uncalled: Vec<_> = hle_bindings
        .iter()
        .filter(|b| !hle_calls.contains_key(&b.index))
        .collect();
    if !uncalled.is_empty() {
        let stateful: Vec<_> = uncalled
            .iter()
            .filter(|b| cellgov_ppu::nid_db::stub_classification(b.nid) != "noop-safe")
            .collect();
        if !stateful.is_empty() {
            println!("  uncalled (non-noop):");
            for b in &stateful {
                let func = cellgov_ppu::nid_db::lookup(b.nid)
                    .map(|(_, f)| f)
                    .unwrap_or("?");
                let class = cellgov_ppu::nid_db::stub_classification(b.nid);
                println!("    {}::{} [{class}]", b.module, func);
            }
        }
        let noop_count = uncalled.len() - stateful.len();
        if noop_count > 0 {
            println!("  uncalled (noop-safe): {noop_count} functions");
        }
    }
}

pub(super) fn print_insn_coverage(insn_coverage: &std::collections::BTreeMap<&'static str, usize>) {
    if !insn_coverage.is_empty() {
        let mut sorted: Vec<_> = insn_coverage.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        println!("instruction_coverage: {} variants executed", sorted.len());
        for (name, count) in &sorted {
            println!("  {name}: {count}x");
        }
    }
}

/// Print the top 20 PCs by hit count with their raw word and decoded
/// mnemonic. When the boot hits max-steps without faulting, the hottest
/// PCs name the busy-loop body that is preventing forward progress.
pub(super) fn print_top_pcs(rt: &Runtime, pc_hits: &std::collections::HashMap<u64, u64>) {
    if pc_hits.is_empty() {
        return;
    }
    let mut sorted: Vec<_> = pc_hits.iter().collect();
    sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(*c));
    println!("top_pcs_by_hit_count:");
    for (pc, count) in sorted.iter().take(20) {
        let raw = fetch_raw_at(rt, **pc)
            .map(|w| format!("0x{w:08x}"))
            .unwrap_or_else(|| "?".to_string());
        let disasm = fetch_raw_at(rt, **pc)
            .and_then(|w| cellgov_ppu::decode::decode(w).ok())
            .map(|insn| insn.variant_name().to_string())
            .unwrap_or_else(|| "?".into());
        println!("  {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
    }
}
