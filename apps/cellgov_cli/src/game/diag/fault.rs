use cellgov_core::{CommitError, Runtime};
use cellgov_exec::UnitStatus;
use cellgov_lv2::PpuThreadState;

use crate::game::stack_walk::append_stack_walk;
use crate::game::step_loop::{block_reason_label, RingCursor, PC_RING_SIZE};

use super::rings::{append_pc_ring_terse, append_pc_ring_with_decode};
use super::{fetch_raw_at, longest_readable_prefix, region_label_at};

pub(in crate::game) fn format_fault(
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
        FAULT_PROGRAM_TRAP, FAULT_UNIMPLEMENTED_INSN, FAULT_UNSUPPORTED_SYSCALL,
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
                FAULT_PROGRAM_TRAP => {
                    let to = code & 0x0000_FFFF;
                    format!("PROGRAM_TRAP (TO=0x{to:02x}) at PC={pc_str}")
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
pub(in crate::game) fn append_register_pointer_dump(
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
pub(in crate::game) fn append_explicit_mem_dump(
    out: &mut String,
    rt: &Runtime,
    ranges: &[(u64, u64)],
) {
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

pub(in crate::game) fn format_commit_fault(
    rt: &Runtime,
    err: &CommitError,
    steps: usize,
    source: cellgov_event::UnitId,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
) -> String {
    let mut out = format!("COMMIT_FAULT at step {steps}: {err:?}");
    // PC-side `FAULT` populates `result.local_diagnostics.fault_regs`
    // before commit runs; the commit path has no equivalent, so dump
    // the source unit's post-step state via `RegisteredUnit::register_dump`.
    if let Some(regs) = rt.registry().get(source).and_then(|u| u.register_dump()) {
        out.push_str(&format!("\n  source unit: {}", source.raw()));
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
        append_register_pointer_dump(&mut out, rt, &regs);
        append_stack_walk(&mut out, rt, &regs);
    }
    append_pc_ring_with_decode(&mut out, rt, pc_ring, pc_cursor);
    out
}

/// Walks `rt.registry()` rather than `Lv2Host::ppu_threads()` so SPU units
/// blocked on mailbox-receive or DMA wait surface alongside PPU threads.
pub(in crate::game) fn format_deadlock(
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

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_exec::FaultRegisterDump;
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
    fn format_commit_fault_includes_error_and_step_and_pc_ring() {
        let rt = rt_with_layout();
        let err = CommitError::PayloadLengthMismatch { effect_index: 3 };
        let mut cursor = RingCursor::new(PC_RING_SIZE);
        let mut ring = [0u64; PC_RING_SIZE];
        for pc in [0x0010_0000u64, 0x0010_0004, 0x0010_0008] {
            let idx = cursor.record();
            ring[idx] = pc;
        }
        let out = format_commit_fault(
            &rt,
            &err,
            1234,
            cellgov_event::UnitId::new(0),
            &ring,
            &cursor,
        );
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
