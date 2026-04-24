//! Diagnostic formatting for `run-game`. Pure formatting: reads state,
//! produces strings or stdout, never mutates guest state.
//!
//! The `pc_ring` parameters threaded through `format_fault`,
//! `format_process_exit`, and `format_max_steps` assume a
//! single-threaded stepper. A concurrent writer would tear reads.

use crate::game::{PC_RING_SIZE, SYSCALL_RING_SIZE};
use cellgov_core::Runtime;

/// Render `bytes` as ASCII, replacing non-printable bytes with `.`.
///
/// Output is always pure ASCII, safe for cp1252/cp437 consoles. The
/// full-slice TTY path uses `from_utf8_lossy` instead to preserve
/// multi-byte UTF-8.
pub(super) fn ascii_safe_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// Fetch a 32-bit big-endian instruction word at `pc`, resolving against
/// any region in the memory map (not just base-0).
pub(super) fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(pc), 4)?;
    let b = rt.memory().read(range)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Label the region containing `[addr, addr+len)`, returning
/// `"<unmapped>"` if the range straddles or escapes every region.
///
/// `len` must match the caller's subsequent read width: querying with
/// `len=1` would label a PC 1-3 bytes before a region boundary as
/// mapped even though the caller's 4-byte fetch would fail.
pub(super) fn region_label_at(rt: &Runtime, addr: u64, len: u64) -> &'static str {
    rt.memory()
        .containing_region(addr, len)
        .map(|r| r.label())
        .unwrap_or("<unmapped>")
}

/// Longest readable prefix of `[buf, buf+len)`. `None` when even the
/// first byte is unmapped. O(log len) memory probes.
pub(super) fn longest_readable_prefix(
    mem: &cellgov_mem::GuestMemory,
    buf: u64,
    len: u64,
) -> Option<(u64, Vec<u8>)> {
    if len == 0 {
        return None;
    }
    let mut lo = 0u64;
    let mut hi = len;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let hit = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), mid)
            .and_then(|r| mem.read(r))
            .is_some();
        if hit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return None;
    }
    let r = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), lo)?;
    let bytes = mem.read(r)?.to_vec();
    Some((lo, bytes))
}

/// Resolve an HLE index into `"{module}::{func}"`. Distinguishes
/// index-OOB, NID-not-in-database, and resolved cases in the output.
pub(super) fn format_hle_idx(idx: u32, hle_bindings: &[cellgov_ppu::prx::HleBinding]) -> String {
    match hle_bindings.get(idx as usize) {
        Some(b) => match cellgov_ppu::nid_db::lookup(b.nid) {
            Some((_, func)) => format!("{}::{func}", b.module),
            None => format!("{}::<unresolved-nid-0x{:08x}>", b.module, b.nid),
        },
        None => format!("<hle-idx-oob {idx}>"),
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
        // "<unmapped>" vs "0x00000000": the zero word decodes as a
        // valid PPC instruction, so unwrap_or(0) would conflate them.
        let raw = fetch_raw_at(rt, pc)
            .map(|w| format!("0x{w:08x}"))
            .unwrap_or_else(|| "<unmapped>".to_string());
        println!(
            "[{steps:>4}] PC=0x{pc:08x}  raw={raw}  yr={:?}",
            result.yield_reason
        );
    }
    if let Some(args) = &result.syscall_args {
        if args[0] >= 0x10000 {
            let idx = (args[0] - 0x10000) as u32;
            println!(
                "       -> HLE #{idx}: {}",
                format_hle_idx(idx, hle_bindings)
            );
        } else if args[0] == 403 {
            let buf = args[2];
            let len = args[3];
            let full = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), len)
                .and_then(|r| rt.memory().read(r));
            match full {
                Some(slice) => {
                    let text = String::from_utf8_lossy(slice);
                    print!("       -> tty: {text}");
                    if !text.ends_with('\n') {
                        println!();
                    }
                }
                None => match longest_readable_prefix(rt.memory(), buf, len) {
                    Some((n, bytes)) => {
                        // ascii_safe_preview strips all control bytes,
                        // so the partial output is always single-line;
                        // the full-slice branch above preserves the
                        // guest's own newline.
                        let text = ascii_safe_preview(&bytes);
                        println!("       -> tty (partial {n}/{len}): {text}");
                    }
                    None => println!("       -> LV2 tty_write (oob, 0/{len} readable)"),
                },
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
                FAULT_PC_OUT_OF_RANGE => {
                    // Raw code kept so low-16 bits surface if the ABI
                    // ever encodes signal there.
                    format!("PC_OUT_OF_RANGE at PC={pc_str} (code=0x{code:08x})")
                }
                FAULT_DECODE_ERROR => {
                    // Three outcomes: no PC in diagnostics, PC unmapped,
                    // PC mapped but word undecodable.
                    let raw_str = match pc {
                        None => "<no-pc>".to_string(),
                        Some(a) => match fetch_raw_at(rt, a) {
                            Some(w) => format!("0x{w:08x}"),
                            None => "<unmapped>".to_string(),
                        },
                    };
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
                    // Dump memory at each GPR that looks pointer-like.
                    // Region-aware so stack pointers (0xD0000000+) are
                    // not dropped.
                    if let Some(regs) = &result.local_diagnostics.fault_regs {
                        for (i, &val) in regs.gprs.iter().enumerate() {
                            if val < 0x1000 {
                                continue;
                            }
                            let Some(range) =
                                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(val), 64)
                            else {
                                s.push_str(&format!(
                                    "\n  [r{i}=0x{val:08x}]: <invalid address range>"
                                ));
                                continue;
                            };
                            let Some(slice) = rt.memory().read(range) else {
                                s.push_str(&format!("\n  [r{i}=0x{val:08x}]: <unreadable>"));
                                continue;
                            };
                            let label = region_label_at(rt, val, 64);
                            s.push_str(&format!("\n  [r{i}=0x{val:08x} ({label})]: "));
                            // If a printable-ASCII prefix leads into
                            // non-printable bytes, count the hidden tail
                            // so an ASCII header on a binary blob does
                            // not erase the rest.
                            let printable = slice
                                .iter()
                                .take_while(|&&b| (0x20..0x7f).contains(&b))
                                .count();
                            if printable >= 4 {
                                let text: String =
                                    slice[..printable].iter().map(|&b| b as char).collect();
                                s.push_str(&format!("{text:?}"));
                                let hidden = slice.len() - printable;
                                if hidden > 0 {
                                    s.push_str(&format!(" (+{hidden} non-printable bytes)"));
                                }
                            } else {
                                for b in &slice[..16.min(slice.len())] {
                                    s.push_str(&format!("{b:02x} "));
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

    // Mini-trace: last N PCs with raw word plus decoded mnemonic.
    // Walking backward through the mnemonics identifies the setter of
    // a bad effective address.
    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            // <unmapped> = fetch failed, <baddec> = word undecodable.
            let (raw, name) = match fetch_raw_at(rt, pc) {
                Some(w) => (
                    format!("0x{w:08x}"),
                    cellgov_ppu::decode::decode(w)
                        .ok()
                        .map(|insn| insn.variant_name().to_string())
                        .unwrap_or_else(|| "<baddec>".into()),
                ),
                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
            };
            out.push_str(&format!("\n    0x{pc:08x}  raw={raw}  {name}"));
        }
    }

    out
}

/// Format the diagnostic artifact for a guest-initiated sys_process_exit.
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

    if let Some(tty) = last_tty {
        out.push_str(&format!(
            "\n  last tty write (fd={}, {} bytes, PC=0x{:08x}):",
            tty.fd,
            tty.raw_bytes.len(),
            tty.call_pc,
        ));
        for chunk in tty.raw_bytes.chunks(16) {
            out.push_str("\n    ");
            for (i, b) in chunk.iter().enumerate() {
                if i == 8 {
                    out.push(' ');
                }
                out.push_str(&format!("{b:02x} "));
            }
        }
        // All-non-printable gets an explicit tag so an all-dots line
        // is not mistaken for a stripped ASCII message.
        let preview = ascii_safe_preview(&tty.raw_bytes);
        let all_nonprintable =
            !tty.raw_bytes.is_empty() && tty.raw_bytes.iter().all(|&b| !(0x20..=0x7E).contains(&b));
        if all_nonprintable {
            out.push_str(&format!(
                "\n  decoded: \"{}\" (all non-printable)",
                preview.trim_end()
            ));
        } else {
            out.push_str(&format!("\n  decoded: \"{}\"", preview.trim_end()));
        }
    }

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
                let name = format_hle_idx(idx, hle_bindings);
                out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
            } else {
                out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
            }
        }
    }

    out
}

/// Format the MAX_STEPS diagnostic with the last PCs and syscalls.
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
                let name = format_hle_idx(idx, hle_bindings);
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
            let (name, class) = match hle_bindings.get(*idx as usize) {
                Some(b) => (
                    format_hle_idx(*idx, hle_bindings),
                    cellgov_ppu::nid_db::stub_classification(b.nid),
                ),
                None => (format!("<hle-idx-oob {idx}>"), "<oob>"),
            };
            println!("    {name}: {count}x [{class}]");
        }
    }

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
                let func = match cellgov_ppu::nid_db::lookup(b.nid) {
                    Some((_, f)) => f.to_string(),
                    None => format!("<unresolved-nid-0x{:08x}>", b.nid),
                };
                let class = cellgov_ppu::nid_db::stub_classification(b.nid);
                println!("    {}::{func} [{class}]", b.module);
            }
        }
        let noop_count = uncalled.len() - stateful.len();
        if noop_count > 0 {
            println!("  uncalled (noop-safe): {noop_count} functions");
        }
    }
}

pub(super) fn print_insn_coverage(insn_coverage: &std::collections::BTreeMap<&'static str, usize>) {
    // Always emit a header so empty output is not mistaken for the
    // feature being disabled.
    if insn_coverage.is_empty() {
        println!("instruction_coverage: none");
        return;
    }
    let mut sorted: Vec<_> = insn_coverage.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    println!("instruction_coverage: {} variants executed", sorted.len());
    for (name, count) in &sorted {
        println!("  {name}: {count}x");
    }
}

/// Report per-unit and summed instruction-shadow hit/miss counts.
/// A rising miss count on a single unit means its fetches have moved
/// outside the shadowed region (PRX bodies above 0x10000000).
pub(super) fn print_shadow_stats(rt: &mut Runtime) {
    let mut per_unit: Vec<(u64, u64, u64)> = Vec::new();
    let mut total_hits = 0u64;
    let mut total_misses = 0u64;
    let mut total_units = 0usize;
    for (id, unit) in rt.registry_mut().iter_mut() {
        total_units += 1;
        let (h, m) = unit.shadow_stats();
        if h + m == 0 {
            continue;
        }
        per_unit.push((id.raw(), h, m));
        total_hits += h;
        total_misses += m;
    }
    let total = total_hits + total_misses;
    if total == 0 {
        println!("shadow: no fetches recorded");
        return;
    }
    let hit_pct = (total_hits as f64 / total as f64) * 100.0;
    let active = per_unit.len();
    // `active` = units that retired at least one instruction;
    // `total_units` = all registered units.
    println!(
        "shadow: {total_hits}/{total} via shadow ({hit_pct:.1}%), {total_misses} decode-on-fetch ({active} active / {total_units} registered)"
    );
    if active > 1 {
        for (unit_id, h, m) in &per_unit {
            let t = h + m;
            let pct = (*h as f64 / t as f64) * 100.0;
            println!("  unit {unit_id}: {h}/{t} via shadow ({pct:.1}%), {m} decode-on-fetch");
        }
    }
}

pub(super) fn print_top_pcs(rt: &Runtime, pc_hits: &std::collections::HashMap<u64, u64>) {
    if pc_hits.is_empty() {
        return;
    }
    let mut sorted: Vec<_> = pc_hits.iter().collect();
    // Stable ordering: descending by count, ascending by PC on ties,
    // so HashMap iteration order does not leak into replay diffs.
    sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
    println!("top_pcs_by_hit_count:");
    for (pc, count) in sorted.iter().take(20) {
        let (raw, disasm) = match fetch_raw_at(rt, **pc) {
            Some(w) => (
                format!("0x{w:08x}"),
                cellgov_ppu::decode::decode(w)
                    .ok()
                    .map(|insn| insn.variant_name().to_string())
                    .unwrap_or_else(|| "<baddec>".into()),
            ),
            None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
        };
        println!("  {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::{GuestMemory, PageSize, Region};
    use cellgov_time::Budget;

    fn rt_with_layout() -> Runtime {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
            Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
        ])
        .unwrap();
        Runtime::new(mem, Budget::new(1), 100)
    }

    #[test]
    fn region_label_at_names_stack_region() {
        let rt = rt_with_layout();
        assert_eq!(region_label_at(&rt, 0xD000_FFF0, 4), "stack");
    }

    #[test]
    fn region_label_at_names_main_region() {
        let rt = rt_with_layout();
        assert_eq!(region_label_at(&rt, 0x0010_0000, 4), "main");
    }

    #[test]
    fn region_label_at_unmapped_addr_is_not_misattributed() {
        let rt = rt_with_layout();
        // 0x80000000 sits between main and stack in the fixture layout.
        assert_eq!(region_label_at(&rt, 0x8000_0000, 4), "<unmapped>");
    }

    #[test]
    fn longest_readable_prefix_returns_none_on_zero_length() {
        let rt = rt_with_layout();
        assert!(longest_readable_prefix(rt.memory(), 0, 0).is_none());
    }

    #[test]
    fn longest_readable_prefix_returns_none_for_entirely_unmapped_buffer() {
        let rt = rt_with_layout();
        assert!(longest_readable_prefix(rt.memory(), 0x8000_0000, 64).is_none());
    }

    #[test]
    fn longest_readable_prefix_finds_region_boundary_exactly() {
        let rt = rt_with_layout();
        // Pinned precondition: a future fixture that maps 0x4000_0000
        // would let this test pass without exercising the boundary.
        assert!(
            longest_readable_prefix(rt.memory(), 0x4000_0000, 1).is_none(),
            "precondition: nothing readable at main's end"
        );
        let buf = 0x4000_0000 - 16;
        let (n, bytes) = longest_readable_prefix(rt.memory(), buf, 64).expect("some prefix");
        assert_eq!(n, 16);
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn longest_readable_prefix_returns_full_len_when_fully_mapped() {
        let rt = rt_with_layout();
        let (n, bytes) = longest_readable_prefix(rt.memory(), 0x0010_0000, 64)
            .expect("fully readable should return Some");
        assert_eq!(n, 64);
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn longest_readable_prefix_single_byte_boundary() {
        let rt = rt_with_layout();
        let buf = 0x4000_0000 - 1;
        let (n, _bytes) = longest_readable_prefix(rt.memory(), buf, 2).expect("single-byte prefix");
        assert_eq!(n, 1);
    }
}
