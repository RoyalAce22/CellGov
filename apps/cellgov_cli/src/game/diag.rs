//! Diagnostic formatting for `run-game`: reads runtime state, produces strings.
//!
//! `pc_ring` readers assume a single-threaded stepper; a concurrent writer
//! would tear reads.

use crate::game::stack_walk::append_stack_walk;
use crate::game::step_loop::{block_reason_label, RingCursor};
use crate::game::{PC_RING_SIZE, SYSCALL_RING_SIZE};
use cellgov_core::{CommitError, Runtime};
use cellgov_exec::UnitStatus;
use cellgov_lv2::PpuThreadState;

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

pub(super) fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(pc), 4)?;
    let b = rt.memory().read(range)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `len` must match the caller's read width; querying with `len=1` mislabels
/// a PC 1-3 bytes before a boundary as mapped when a 4-byte fetch would fail.
pub(super) fn region_label_at(rt: &Runtime, addr: u64, len: u64) -> &'static str {
    rt.memory()
        .containing_region(addr, len)
        .map(|r| r.label())
        .unwrap_or("<unmapped>")
}

/// Longest readable prefix of `[buf, buf+len)` via O(log len) probes.
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

pub(super) fn format_hle_idx(idx: u32, hle_bindings: &[cellgov_ppu::prx::HleBinding]) -> String {
    match hle_bindings.get(idx as usize) {
        Some(b) => match cellgov_ps3_abi::nid::lookup(b.nid) {
            Some((_, func)) => format!("{}::{func}", b.module),
            None => format!("{}::<unresolved-nid-0x{:08x}>", b.module, b.nid),
        },
        None => format!("<hle-idx-oob {idx}>"),
    }
}

pub(super) struct TtyCapture {
    pub(super) fd: u32,
    pub(super) raw_bytes: Vec<u8>,
    pub(super) call_pc: u64,
}

pub(super) struct ProcessExitInfo {
    pub(super) code: u32,
    pub(super) call_pc: u64,
}

pub(super) fn print_trace_line(
    rt: &Runtime,
    unit: cellgov_event::UnitId,
    result: &cellgov_exec::ExecutionStepResult,
    steps: usize,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    if let Some(pc) = result.local_diagnostics.pc {
        // Zero decodes as a valid PPC instruction; distinguish unmapped from a real zero word.
        let raw = fetch_raw_at(rt, pc)
            .map(|w| format!("0x{w:08x}"))
            .unwrap_or_else(|| "<unmapped>".to_string());
        println!(
            "[{steps:>4}] u{} PC=0x{pc:08x}  raw={raw}  yr={:?}",
            unit.raw(),
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
    pc_cursor: &RingCursor,
    dump_mem_fault_ranges: &[(u64, u64)],
) -> String {
    let pc = result.local_diagnostics.pc;
    let pc_str = pc
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    use cellgov_ppu::{
        FAULT_DEBUG_BREAK, FAULT_DECODE_ERROR, FAULT_INVALID_ADDRESS, FAULT_PC_OUT_OF_RANGE,
        FAULT_UNIMPLEMENTED_INSN, FAULT_UNSUPPORTED_SYSCALL,
    };
    let detail = match fault {
        cellgov_effects::FaultKind::Guest(code) => {
            let fault_type = code & 0xFFFF_0000;
            match fault_type {
                FAULT_PC_OUT_OF_RANGE => {
                    format!("PC_OUT_OF_RANGE at PC={pc_str} (code=0x{code:08x})")
                }
                FAULT_DECODE_ERROR => {
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
                FAULT_UNIMPLEMENTED_INSN => {
                    let xo = code & 0x0000_FFFF;
                    format!("UNIMPLEMENTED_INSN (xo=0x{xo:x}) at PC={pc_str}")
                }
                FAULT_DEBUG_BREAK => format!("DEBUG_BREAK at PC={pc_str}"),
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
            "\n    LR=0x{:016x}  CTR=0x{:016x}  XER=0x{:016x}  CR=0x{:08x}",
            regs.lr, regs.ctr, regs.xer, regs.cr
        ));
        append_register_pointer_dump(&mut out, rt, regs);
        append_stack_walk(&mut out, rt, regs);
    }

    append_explicit_mem_dump(&mut out, rt, dump_mem_fault_ranges);
    append_pc_ring_with_decode(&mut out, rt, pc_ring, pc_cursor);

    out
}

/// Dump 64 bytes at any GPR/LR/CTR value >= 0x1000 (plausible pointer).
pub(super) fn append_register_pointer_dump(
    out: &mut String,
    rt: &Runtime,
    regs: &cellgov_exec::FaultRegisterDump,
) {
    let mut emitted = false;
    for (i, &val) in regs.gprs.iter().enumerate() {
        if val < 0x1000 {
            continue;
        }
        if !emitted {
            out.push_str("\n  register pointers:");
            emitted = true;
        }
        append_register_pointer_line(out, rt, &format!("r{i}"), val);
    }
    for (label, val) in [("LR", regs.lr), ("CTR", regs.ctr)] {
        if val < 0x1000 {
            continue;
        }
        if !emitted {
            out.push_str("\n  register pointers:");
            emitted = true;
        }
        append_register_pointer_line(out, rt, label, val);
    }
}

fn append_register_pointer_line(out: &mut String, rt: &Runtime, label: &str, val: u64) {
    let Some(range) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(val), 64) else {
        out.push_str(&format!(
            "\n    [{label}=0x{val:016x}]: <invalid address range>"
        ));
        return;
    };
    let Some(slice) = rt.memory().read(range) else {
        out.push_str(&format!("\n    [{label}=0x{val:016x}]: <unreadable>"));
        return;
    };
    let region = region_label_at(rt, val, 64);
    out.push_str(&format!("\n    [{label}=0x{val:016x} ({region})]: "));
    let printable = slice
        .iter()
        .take_while(|&&b| (0x20..0x7f).contains(&b))
        .count();
    if printable >= 4 {
        let text: String = slice[..printable].iter().map(|&b| b as char).collect();
        out.push_str(&format!("{text:?}"));
        let hidden = slice.len() - printable;
        if hidden > 0 {
            out.push_str(&format!(" (+{hidden} non-printable bytes)"));
        }
    } else {
        for b in &slice[..16.min(slice.len())] {
            out.push_str(&format!("{b:02x} "));
        }
    }
}

/// Render `--dump-mem-fault` ranges as hex+ASCII; partial-straddle ranges emit the readable prefix.
pub(super) fn append_explicit_mem_dump(out: &mut String, rt: &Runtime, ranges: &[(u64, u64)]) {
    if ranges.is_empty() {
        return;
    }
    out.push_str("\n  explicit mem dumps:");
    for &(addr, len) in ranges {
        let Some(range) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), len)
        else {
            out.push_str(&format!(
                "\n    mem[0x{addr:016x}+{len}]: <invalid address range>"
            ));
            continue;
        };
        let region = region_label_at(rt, addr, len);
        match rt.memory().read(range) {
            Some(slice) => {
                out.push_str(&format!(
                    "\n    mem[0x{addr:016x} ({region}), {len} bytes]:"
                ));
                append_hex_ascii_block(out, addr, slice);
            }
            None => match longest_readable_prefix(rt.memory(), addr, len) {
                Some((prefix_len, bytes)) => {
                    let tail = len - prefix_len;
                    out.push_str(&format!(
                        "\n    mem[0x{addr:016x} ({region}), {prefix_len}/{len} bytes (tail {tail} unmapped)]:"
                    ));
                    append_hex_ascii_block(out, addr, &bytes);
                }
                None => out.push_str(&format!(
                    "\n    mem[0x{addr:016x} ({region}), {len} bytes]: <unmapped>"
                )),
            },
        }
    }
}

/// `xxd -g1` style: 16-byte rows of hex + ASCII, offsets relative to `base_addr`.
fn append_hex_ascii_block(out: &mut String, base_addr: u64, bytes: &[u8]) {
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let row_addr = base_addr + (row as u64) * 16;
        out.push_str(&format!("\n      +0x{:03x} (0x{row_addr:016x}):", row * 16));
        for b in chunk {
            out.push_str(&format!(" {b:02x}"));
        }
        for _ in chunk.len()..16 {
            out.push_str("   ");
        }
        out.push_str("  |");
        for &b in chunk {
            out.push(if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        out.push('|');
    }
}

pub(super) fn format_commit_fault(
    rt: &Runtime,
    err: &CommitError,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) -> String {
    let mut out = format!("COMMIT_FAULT at step {steps}: {err:?}");
    append_pc_ring_with_decode(&mut out, rt, pc_ring, pc_cursor);
    out
}

/// Walks `rt.registry()` rather than `Lv2Host::ppu_threads()` so SPU units
/// blocked on mailbox-receive or DMA wait surface alongside PPU threads.
pub(super) fn format_deadlock(
    rt: &Runtime,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) -> String {
    let mut out = format!("DEADLOCK after {steps} steps:");
    let mut blocked_count = 0usize;
    let blocked_ids: Vec<_> = rt
        .registry()
        .iter()
        .filter_map(|(id, _)| {
            (rt.registry().effective_status(id) == Some(UnitStatus::Blocked)).then_some(id)
        })
        .collect();
    for unit_id in blocked_ids {
        blocked_count += 1;
        match rt.lv2_host().ppu_thread_for_unit(unit_id) {
            Some(thread) => {
                let label = match &thread.state {
                    PpuThreadState::Blocked(reason) => block_reason_label(reason),
                    other => format!("(unit Blocked but PPU thread state is {other:?})"),
                };
                out.push_str(&format!(
                    "\n  unit {} (PPU thread {}): {}",
                    unit_id.raw(),
                    thread.id.raw(),
                    label,
                ));
            }
            None => {
                out.push_str(&format!(
                    "\n  unit {} (no LV2 PPU thread record; SPU or pre-LV2 unit)",
                    unit_id.raw(),
                ));
            }
        }
    }
    if blocked_count == 0 {
        // AllBlocked fires only when a unit is Blocked; empty walk means
        // registry status and effective_status disagreed.
        out.push_str("\n  (no Blocked units in registry; AllBlocked may have raced a wake)");
    } else {
        out.push_str(&format!("\n  {blocked_count} blocked unit(s) total"));
    }
    append_pc_ring_terse(&mut out, pc_ring, pc_cursor);
    out
}

pub(super) fn append_orphan_exit_info(
    diagnostic: &mut String,
    last_exit: Option<&ProcessExitInfo>,
) {
    let Some(exit) = last_exit else {
        return;
    };
    diagnostic.push_str(&format!(
        "\n  note: stale exit info captured before terminal verdict (code={}, PC=0x{:08x})",
        exit.code, exit.call_pc,
    ));
}

fn append_pc_ring_with_decode(
    out: &mut String,
    rt: &Runtime,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) {
    let filled = pc_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for i in pc_cursor.iter_indices() {
        let pc = pc_ring[i];
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

fn append_pc_ring_terse(out: &mut String, pc_ring: &[u64; PC_RING_SIZE], pc_cursor: &RingCursor) {
    let filled = pc_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} PCs:"));
    for i in pc_cursor.iter_indices() {
        let pc = pc_ring[i];
        out.push_str(&format!("\n    0x{pc:08x}"));
    }
}

fn append_syscall_ring(
    out: &mut String,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    let filled = syscall_cursor.filled();
    if filled == 0 {
        return;
    }
    out.push_str(&format!("\n  last {filled} syscalls:"));
    for i in syscall_cursor.iter_indices() {
        let (nr, pc) = syscall_ring[i];
        if nr >= 0x10000 {
            let idx = (nr - 0x10000) as u32;
            let name = format_hle_idx(idx, hle_bindings);
            out.push_str(&format!("\n    HLE {name} at 0x{pc:08x}"));
        } else {
            out.push_str(&format!("\n    LV2 #{nr} at 0x{pc:08x}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn format_process_exit(
    exit: &ProcessExitInfo,
    last_tty: Option<&TtyCapture>,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
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
        // Tag all-non-printable so a dots-only line is not mistaken for stripped ASCII.
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

    append_pc_ring_terse(&mut out, pc_ring, pc_cursor);
    append_syscall_ring(&mut out, syscall_ring, syscall_cursor, hle_bindings);
    out
}

pub(super) fn format_max_steps(
    rt: &Runtime,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) -> String {
    let mut out = format!("MAX_STEPS after {} steps", steps);
    append_unit_state_summary(&mut out, rt);
    append_pc_ring_terse(&mut out, pc_ring, pc_cursor);
    append_syscall_ring(&mut out, syscall_ring, syscall_cursor, hle_bindings);
    out
}

/// One line per unit: id, effective status, LV2 PPU thread state if any.
pub(super) fn append_unit_state_summary(out: &mut String, rt: &Runtime) {
    let ids: Vec<_> = rt.registry().ids().collect();
    out.push_str(&format!("\n  units: {} total", ids.len()));
    for unit_id in ids {
        let status = rt
            .registry()
            .effective_status(unit_id)
            .map(|s| format!("{s:?}"))
            .unwrap_or_else(|| "<missing>".to_string());
        let thread_label = match rt.lv2_host().ppu_thread_for_unit(unit_id) {
            Some(thread) => match &thread.state {
                PpuThreadState::Blocked(reason) => {
                    format!(
                        "PPU thread {} entry=0x{:x} {}",
                        thread.id.raw(),
                        thread.attrs.entry,
                        block_reason_label(reason)
                    )
                }
                other => format!(
                    "PPU thread {} entry=0x{:x} {:?}",
                    thread.id.raw(),
                    thread.attrs.entry,
                    other
                ),
            },
            None => "no LV2 PPU thread record (SPU or pre-LV2)".to_string(),
        };
        let pending = match rt.syscall_responses().peek(unit_id) {
            Some(p) => format!(" pending={p:?}"),
            None => String::new(),
        };
        out.push_str(&format!(
            "\n    unit {} status={} {}{}",
            unit_id.raw(),
            status,
            thread_label,
            pending,
        ));
    }
}

pub(super) fn print_hle_summary(
    hle_calls: &std::collections::BTreeMap<u32, usize>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    let called_count = hle_calls.len();
    let total_count = hle_bindings.len();
    let uncalled_count = total_count - called_count.min(total_count);
    println!("hle_imports: {total_count} bound, {called_count} called, {uncalled_count} uncalled");

    use cellgov_ps3_abi::nid::StubClass;
    if !hle_calls.is_empty() {
        println!("  called:");
        for (idx, count) in hle_calls {
            let (name, class) = match hle_bindings.get(*idx as usize) {
                Some(b) => (
                    format_hle_idx(*idx, hle_bindings),
                    cellgov_ps3_abi::nid::stub_classification(b.nid).as_str(),
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
            .filter(|b| cellgov_ps3_abi::nid::stub_classification(b.nid) != StubClass::NoopSafe)
            .collect();
        if !stateful.is_empty() {
            println!("  uncalled (non-noop):");
            for b in &stateful {
                let func = match cellgov_ps3_abi::nid::lookup(b.nid) {
                    Some((_, f)) => f.to_string(),
                    None => format!("<unresolved-nid-0x{:08x}>", b.nid),
                };
                let class = cellgov_ps3_abi::nid::stub_classification(b.nid).as_str();
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

/// A rising per-unit miss count means its fetches moved outside the
/// shadowed region (PRX bodies above 0x10000000).
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
    // Tie-break by PC so HashMap iteration order does not leak into replay diffs.
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

    #[test]
    fn append_orphan_exit_info_is_noop_when_none() {
        let mut s = String::from("FAULT at step 100");
        append_orphan_exit_info(&mut s, None);
        assert_eq!(s, "FAULT at step 100");
    }

    #[test]
    fn append_orphan_exit_info_appends_code_and_pc_when_some() {
        let mut s = String::from("FAULT at step 100");
        append_orphan_exit_info(
            &mut s,
            Some(&ProcessExitInfo {
                code: 0x42,
                call_pc: 0x10ab_cdef,
            }),
        );
        assert!(s.contains("code=66"), "got {s}");
        assert!(s.contains("PC=0x10abcdef"), "got {s}");
        assert!(s.contains("stale exit info"), "got {s}");
    }

    #[test]
    fn format_commit_fault_includes_error_and_step_and_pc_ring() {
        let rt = rt_with_layout();
        let err = CommitError::PayloadLengthMismatch { effect_index: 3 };
        let mut cursor = RingCursor::new(PC_RING_SIZE);
        let mut ring = [0u64; PC_RING_SIZE];
        for pc in [0x0010_0000u64, 0x0010_0004, 0x0010_0008] {
            let idx = cursor.record();
            ring[idx] = pc;
        }
        let out = format_commit_fault(&rt, &err, 1234, &ring, &cursor);
        assert!(out.starts_with("COMMIT_FAULT at step 1234"), "got {out}");
        assert!(out.contains("PayloadLengthMismatch"), "got {out}");
        assert!(out.contains("last 3 PCs:"), "got {out}");
        assert!(out.contains("0x00100000"), "got {out}");
    }

    #[test]
    fn format_deadlock_with_empty_registry_flags_drift() {
        let rt = rt_with_layout();
        let cursor = RingCursor::new(PC_RING_SIZE);
        let ring = [0u64; PC_RING_SIZE];
        let out = format_deadlock(&rt, 99, &ring, &cursor);
        assert!(out.starts_with("DEADLOCK after 99 steps:"), "got {out}");
        assert!(
            out.contains("no Blocked units in registry") && out.contains("AllBlocked"),
            "expected drift note in {out}",
        );
    }

    #[test]
    fn format_deadlock_dumps_ppu_unit_with_lv2_reason_and_spu_unit_without() {
        use cellgov_lv2::{GuestBlockReason, PpuThreadAttrs, PpuThreadState};
        use cellgov_testkit::world::CountingUnit;

        let mut rt = rt_with_layout();
        let unit_a = rt
            .registry_mut()
            .register_with(|id| CountingUnit::new(id, 100));
        let unit_b = rt
            .registry_mut()
            .register_with(|id| CountingUnit::new(id, 100));

        rt.registry_mut()
            .set_status_override(unit_a, UnitStatus::Blocked);
        rt.registry_mut()
            .set_status_override(unit_b, UnitStatus::Blocked);

        let attrs = PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        };
        let ppu_id_a = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(unit_a, attrs)
            .unwrap();
        rt.lv2_host_mut()
            .ppu_threads_mut()
            .get_mut(ppu_id_a)
            .unwrap()
            .state = PpuThreadState::Blocked(GuestBlockReason::WaitingOnLwMutex { id: 7 });
        // unit_b has no PpuThread record (models SPU/non-PPU shape).

        let cursor = RingCursor::new(PC_RING_SIZE);
        let ring = [0u64; PC_RING_SIZE];
        let out = format_deadlock(&rt, 42, &ring, &cursor);
        assert!(out.contains("DEADLOCK after 42 steps:"), "got {out}");
        assert!(
            out.contains(&format!("unit {} (PPU thread", unit_a.raw())),
            "PPU unit not labeled: {out}",
        );
        assert!(out.contains("WaitingOnLwMutex(id=7)"), "got {out}");
        assert!(
            out.contains(&format!("unit {} (no LV2 PPU thread record", unit_b.raw())),
            "SPU-shaped unit not labeled: {out}",
        );
        assert!(out.contains("2 blocked unit(s) total"), "got {out}");
    }

    use cellgov_exec::FaultRegisterDump;

    fn write_bytes(rt: &mut Runtime, addr: u64, bytes: &[u8]) {
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
                .unwrap();
        rt.memory_mut().apply_commit(range, bytes).unwrap();
    }

    fn fault_regs() -> FaultRegisterDump {
        FaultRegisterDump {
            gprs: [0; 32],
            lr: 0,
            ctr: 0,
            xer: 0,
            cr: 0,
        }
    }

    #[test]
    fn append_register_pointer_dump_emits_for_pointer_gprs_and_skips_zero() {
        let mut rt = rt_with_layout();
        write_bytes(&mut rt, 0x0010_0000, b"hello world\x00\x00\x00\x00\x00");
        let mut regs = fault_regs();
        regs.gprs[3] = 0x0010_0000;
        regs.gprs[4] = 0;

        let mut out = String::new();
        append_register_pointer_dump(&mut out, &rt, &regs);
        assert!(out.contains("register pointers:"), "got {out}");
        assert!(out.contains("[r3=0x0000000000100000 (main)]"), "got {out}");
        assert!(out.contains("\"hello world\""), "got {out}");
        assert!(!out.contains("[r4="), "zero r4 leaked: {out}");
    }

    #[test]
    fn append_register_pointer_dump_handles_unmapped_pointer() {
        let rt = rt_with_layout();
        let mut regs = fault_regs();
        regs.gprs[7] = 0x8000_0000;

        let mut out = String::new();
        append_register_pointer_dump(&mut out, &rt, &regs);
        assert!(out.contains("[r7=0x0000000080000000"), "got {out}");
        assert!(out.contains("<unreadable>"), "got {out}");
    }

    #[test]
    fn append_register_pointer_dump_includes_lr_and_ctr() {
        let mut rt = rt_with_layout();
        write_bytes(&mut rt, 0x0020_0000, &[0xab; 64]);
        let mut regs = fault_regs();
        regs.lr = 0x0020_0000;
        regs.ctr = 0x0020_0000;

        let mut out = String::new();
        append_register_pointer_dump(&mut out, &rt, &regs);
        assert!(out.contains("[LR=0x0000000000200000"), "got {out}");
        assert!(out.contains("[CTR=0x0000000000200000"), "got {out}");
    }

    #[test]
    fn append_register_pointer_dump_emits_nothing_when_no_pointers() {
        let rt = rt_with_layout();
        let regs = fault_regs();

        let mut out = String::new();
        append_register_pointer_dump(&mut out, &rt, &regs);
        assert!(out.is_empty(), "expected silence, got {out}");
    }

    #[test]
    fn append_explicit_mem_dump_renders_hex_and_ascii() {
        let mut rt = rt_with_layout();
        write_bytes(&mut rt, 0x0010_0000, b"ABCDEFGHIJKLMNOP\x00\x01\x02\x03");
        let ranges = [(0x0010_0000u64, 20u64)];

        let mut out = String::new();
        append_explicit_mem_dump(&mut out, &rt, &ranges);
        assert!(out.contains("explicit mem dumps:"), "got {out}");
        assert!(
            out.contains("mem[0x0000000000100000 (main), 20 bytes]:"),
            "got {out}"
        );
        // Hex bytes for "ABCDEFGHIJKLMNOP".
        assert!(
            out.contains("41 42 43 44 45 46 47 48"),
            "hex row missing in {out}"
        );
        assert!(out.contains("|ABCDEFGHIJKLMNOP|"), "got {out}");
    }

    #[test]
    fn append_explicit_mem_dump_invalid_range_does_not_panic() {
        let rt = rt_with_layout();
        let ranges = [(u64::MAX, 64u64)];

        let mut out = String::new();
        append_explicit_mem_dump(&mut out, &rt, &ranges);
        assert!(out.contains("explicit mem dumps:"), "got {out}");
        assert!(out.contains("<invalid address range>"), "got {out}");
    }

    #[test]
    fn append_explicit_mem_dump_unmapped_falls_back_to_unmapped_marker() {
        let rt = rt_with_layout();
        let ranges = [(0x8000_0000u64, 64u64)];

        let mut out = String::new();
        append_explicit_mem_dump(&mut out, &rt, &ranges);
        assert!(out.contains("<unmapped>"), "got {out}");
    }

    #[test]
    fn append_explicit_mem_dump_partial_straddle_shows_prefix() {
        let mut rt = rt_with_layout();
        let buf = 0x4000_0000 - 8;
        write_bytes(&mut rt, buf, &[0x55; 8]);
        let ranges = [(buf, 32u64)];

        let mut out = String::new();
        append_explicit_mem_dump(&mut out, &rt, &ranges);
        assert!(out.contains("8/32 bytes (tail 24 unmapped)"), "got {out}");
        assert!(out.contains("55 55 55 55 55 55 55 55"), "got {out}");
    }

    #[test]
    fn append_explicit_mem_dump_empty_ranges_emits_nothing() {
        let rt = rt_with_layout();
        let mut out = String::new();
        append_explicit_mem_dump(&mut out, &rt, &[]);
        assert!(out.is_empty(), "expected silence on empty, got {out}");
    }
}
