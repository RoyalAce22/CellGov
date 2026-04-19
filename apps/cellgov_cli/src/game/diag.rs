//! Diagnostic formatting and printing for `run-game`.
//!
//! Split out of `game.rs` to keep the core boot driver manageable.
//! Every function here is pure formatting: it reads state and produces
//! a String or stdout output, and does not mutate guest state.
//!
//! ## pc_ring invariant
//!
//! `format_fault`, `format_process_exit`, and `format_max_steps` all
//! take a `pc_ring: &[u64; PC_RING_SIZE]` borrowed from the CLI's
//! step loop in `game.rs`. That step loop is single-threaded; the
//! multi-PPU threading that CellGov supports lives inside the
//! runtime, not at this driver layer. All three formatters rely on
//! that invariant for correctness -- a second concurrent stepper
//! writing the ring would cause torn reads that silently lie in the
//! mini-trace.
//!
//! TODO: before any multi-stepper refactor, replace the three
//! `&[u64; PC_RING_SIZE]` parameters with a `PcRing` newtype that
//! is `!Send` (e.g. via `PhantomData<Cell<()>>`) so the type
//! system rejects moving the ring into a worker thread. Doing
//! this at the module level rather than on a single formatter
//! keeps the refactor atomic across all three readers.

use crate::game::{PC_RING_SIZE, SYSCALL_RING_SIZE};
use cellgov_core::Runtime;

/// Render `bytes` as an ASCII-safe preview string.
///
/// Each byte is either passed through (printable ASCII: 0x20..=0x7E)
/// or replaced with `.`. Result is always pure ASCII and contains no
/// control characters or Unicode replacement glyphs, so Windows
/// console renderings (cp1252 / cp437) stay clean. Intended for the
/// partial-read TTY branch and post-run "decoded" summaries when the
/// guest writes binary payloads.
///
/// Deliberately asymmetric with the full-slice TTY path: when the
/// guest's own `(buf, len)` read succeeds in full, we render via
/// `String::from_utf8_lossy` to preserve legitimate multi-byte UTF-8
/// (Japanese text, box-drawing characters, etc.) at the cost of
/// occasional U+FFFD glyphs on invalid bytes. The partial branch
/// and the crash-dump "decoded" line trade fidelity for guaranteed
/// ASCII because those are already in failure-mode output where
/// diagnostic cleanliness matters more than preserving the guest's
/// intended rendering.
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

/// Fetch a 32-bit big-endian instruction word from guest memory at `pc`.
///
/// Region-aware: resolves `pc` against every region in the memory map,
/// not just the base-0 region. This keeps backtrace printing honest
/// when a function pointer leaks into the stack region (0xD0000000+)
/// or any other auxiliary region.
pub(super) fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(pc), 4)?;
    let b = rt.memory().read(range)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Label the region containing `[addr, addr+len)`, for human-readable
/// fault context. The `len` argument must match the operation the
/// caller is about to perform (4 for an instruction fetch, 64 for a
/// DEBUG_BREAK dump) so the helper's notion of "mapped" agrees with
/// the subsequent read. Querying with `len=1` would label a PC that
/// sits 1-3 bytes before a region boundary as "inside region X" even
/// though the 4-byte fetch fails.
///
/// Returns `"<unmapped>"` if the range does not fall entirely in any
/// single region.
pub(super) fn region_label_at(rt: &Runtime, addr: u64, len: u64) -> &'static str {
    rt.memory()
        .containing_region(addr, len)
        .map(|r| r.label())
        .unwrap_or("<unmapped>")
}

/// Binary-search for the longest prefix of `[buf, buf+len)` that is
/// fully readable from guest memory. Returns the prefix length plus
/// its bytes. `None` means even the single starting byte is unmapped.
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

/// Resolve an HLE index captured from a trace or syscall-ring entry
/// into a printable `"{module}::{func}"` string. Distinguishes three
/// failure modes so operators can tell "unbound HLE" apart from
/// "index out of range" (which usually means a corrupted capture or
/// a rebuilt bindings table) and "NID not in database".
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
        // "<unmapped>" distinguishes a failed 4-byte fetch from a PC
        // that genuinely contains the word 0x00000000 (which decodes
        // as a valid instruction on PPC). unwrap_or(0) would conflate
        // the two.
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
                        // Route partial-prefix bytes through
                        // ascii_safe_preview to avoid U+FFFD on
                        // Windows cp1252/cp437 when the truncation
                        // point lands mid-UTF-8. The full-slice path
                        // (guest-chosen length) is less likely to
                        // hit this, but consistency beats risk here.
                        //
                        // Newline strategy differs from the full-slice
                        // path by design: ascii_safe_preview strips
                        // all control bytes including '\n', so the
                        // partial output is always a single line and
                        // println!'s unconditional trailing '\n' is
                        // exactly one. The full-slice path uses
                        // print! + conditional println!() because it
                        // preserves the guest's own '\n' (or absence)
                        // in the payload.
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
                    // Include the raw code so any future ABI change that
                    // starts encoding bits into the low 16 is visible
                    // rather than silently discarded.
                    format!("PC_OUT_OF_RANGE at PC={pc_str} (code=0x{code:08x})")
                }
                FAULT_DECODE_ERROR => {
                    // Three-way distinction: PC absent from diagnostics
                    // (<no-pc>), PC present but unmapped (<unmapped>),
                    // PC present and mapped but the word did not decode
                    // (raw hex shown). Collapsing these was how the
                    // old "?" rendering lost signal.
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
                    // Dump memory at each GPR that looks like a guest pointer.
                    // Region-aware: queries every region by address so a
                    // GPR holding a stack address (0xD0000000+) is dumped
                    // from the stack region, not silently dropped because
                    // it falls outside the base-0 region. Skipped registers
                    // still get a one-line marker so "r5 dumped, r6 missing"
                    // is not an unexplained hole in the dump.
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
                            // Show printable ASCII if it looks like a string.
                            // If non-printable bytes follow the printable
                            // prefix, note how many are hidden so a 5-byte
                            // ASCII header on a binary blob does not silently
                            // erase the 59 bytes after it.
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
    //
    // pc_ring concurrency invariant: see the module-level doc
    // comment at the top of this file.
    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            // Distinguish fetch failure (<unmapped>) from decode
            // failure (<baddec>) so the backtrace tells an operator
            // which path went wrong. Previously both rendered "?".
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
        // ASCII-safe preview. Non-printable bytes render as `.`
        // so binary payloads do not leak control chars or U+FFFD
        // replacements to the terminal. When every byte is
        // non-printable, mark the preview explicitly so an
        // all-dots line does not look like a stripped ASCII
        // message -- the hex dump above still carries the real
        // bytes, but the "decoded:" line needs its own signal.
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
                let name = format_hle_idx(idx, hle_bindings);
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
                // nid_db miss is distinct from print-level "?": tag
                // it so an absent NID entry is legible as a database
                // gap rather than a generic unknown.
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
    // Always emit the header so "no output" is never mistaken for
    // "coverage tallying disabled" or "the feature has been removed".
    // This is always-on diagnostic data; an empty map is a real
    // (if unusual) result.
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

/// Print the top 20 PCs by hit count with their raw word and decoded
/// mnemonic. When the boot hits max-steps without faulting, the hottest
/// PCs name the busy-loop body that is preventing forward progress.
/// Report the instruction-shadow hit/miss counts per unit plus the
/// summed total. A rising miss count on any single unit signals that
/// code that unit is fetching has moved outside the base-0 shadow
/// (PRX bodies above 0x10000000, trampolines past the shadow end)
/// and the fast path is quietly regressing even though correctness
/// holds. With multi-PPU threading live, a worker thread fetching
/// predominantly from an unshadowed region would otherwise have its
/// misses drowned by a mostly-shadowed primary -- the per-unit
/// lines keep that visible.
pub(super) fn print_shadow_stats(rt: &mut Runtime) {
    // Derive registered and active counts from the same iteration
    // pass. Reading `registry().len()` separately from
    // `registry_mut().iter_mut()` would be correct today -- nothing
    // between the two calls mutates the registry -- but brittle if
    // someone later inserts a registry-mutating call between them
    // (reentrancy, not threading).
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
        // Explicit "none" so a zero-fetch run is distinguishable
        // from "stats not plumbed" or "feature disabled".
        println!("shadow: no fetches recorded");
        return;
    }
    let hit_pct = (total_hits as f64 / total as f64) * 100.0;
    let active = per_unit.len();
    // `active` counts units that retired at least one instruction;
    // `total_units` includes registered-but-silent ones. Reporting
    // both lets an operator distinguish "1 active unit out of 4" from
    // "1 active unit out of 1" without rerunning with --profile.
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
    // Stable order: descending by hit count, ascending by PC on
    // ties. Without the PC tiebreak, HashMap iteration order
    // leaks into the display and replay diffs show spurious
    // reorderings whenever multiple PCs share a hit count.
    sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
    println!("top_pcs_by_hit_count:");
    for (pc, count) in sorted.iter().take(20) {
        // Distinguish fetch failure from decode failure (same
        // reason as the format_fault mini-trace).
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
        // A pointer that looks like a primary-thread stack address must
        // resolve to the "stack" region label, not "main" or
        // "<unmapped>". Backtrace and dump-mem helpers depend on this
        // routing -- legacy backtrace helpers assumed "high address = top
        // of contiguous memory" and silently dropped 0xD000xxxx values.
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
        // 0x80000000 is between main (ends at 0x40000000) and stack
        // (starts at 0xD0000000). Must surface as <unmapped>, not be
        // silently routed to either neighbor.
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
        // 0x80000000 is not in any region. Even a 1-byte read must
        // return None; otherwise the "oob, 0/N readable" branch in
        // print_trace_line never fires.
        assert!(longest_readable_prefix(rt.memory(), 0x8000_0000, 64).is_none());
    }

    #[test]
    fn longest_readable_prefix_finds_region_boundary_exactly() {
        let rt = rt_with_layout();
        // main ends at 0x4000_0000. Start 16 bytes before the end
        // and ask for 64 bytes; the helper must return exactly the
        // 16-byte prefix that lies inside main. This is the
        // partial-TTY "crossed into unmapped space" scenario.
        // Precondition: nothing is readable at or after main's end,
        // pinned explicitly so a future fixture that adds a region
        // at 0x4000_0000 turns this test red instead of silently
        // passing without exercising the boundary case.
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
        // A 64-byte read inside main succeeds in full; helper must
        // return the full requested length, not some shorter prefix.
        let (n, bytes) = longest_readable_prefix(rt.memory(), 0x0010_0000, 64)
            .expect("fully readable should return Some");
        assert_eq!(n, 64);
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn longest_readable_prefix_single_byte_boundary() {
        let rt = rt_with_layout();
        // Exactly one byte at the last valid address; requesting two
        // must return one. Exercises the lo=0, hi=1 path where mid=1
        // misses on the second-byte test.
        let buf = 0x4000_0000 - 1;
        let (n, _bytes) = longest_readable_prefix(rt.memory(), buf, 2).expect("single-byte prefix");
        assert_eq!(n, 1);
    }
}
