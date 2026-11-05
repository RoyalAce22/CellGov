//! The fault report: what a unit was doing when it stopped, and the
//! guest state around the address it stopped on.

use cellgov_core::{CommitError, Runtime};
use cellgov_exec::UnitStatus;
use cellgov_lv2::PpuThreadState;
use cellgov_mem::GuestMemory;

use crate::stack_walk::append_stack_walk;
use crate::step_loop::{block_reason_label, PcRing};

use super::rings::{append_pc_ring_terse, append_pc_ring_with_decode};
use super::{fetch_raw_at, longest_readable_prefix, region_label_at};

/// The report line for a PPU fault of `class`: the PPE interrupt with
/// its SRR0, SRR1 cause bits, DSISR and DAR, or the CellGov stop.
///
/// `raw` is the word at the fault's PC, as text.
fn ppu_fault_line(
    class: cellgov_ppu::PpuFaultClass,
    code: u32,
    diag: &cellgov_exec::LocalDiagnostics,
    raw: &str,
) -> String {
    use cellgov_ppu::PpuFaultClass as C;
    let hex = |v: Option<u64>| v.map_or_else(|| "?".to_string(), |a| format!("0x{a:08x}"));
    let pc = hex(diag.pc);
    let record = diag.interrupt.as_ref();
    let srr0 = hex(record.map(|r| r.srr0));
    let dar = hex(record.and_then(|r| r.dar));
    let dsisr = record
        .and_then(|r| r.dsisr)
        .map_or_else(|| "?".to_string(), |d| format!("0x{d:08x}"));
    let access = match diag.data_access.map(|a| a.store) {
        Some(true) => "store",
        Some(false) => "load",
        None => "access",
    };
    // SRR1 bits `first..=last`, ISA-numbered from the most significant bit.
    let srr1_bits = |first: u32, last: u32| {
        let width = (last - first + 1) as usize;
        record.map_or_else(
            || "?".to_string(),
            |r| {
                format!(
                    "0b{:0width$b}",
                    (r.srr1 >> (63 - last)) & ((1 << width) - 1)
                )
            },
        )
    };
    let program = |reason: &str| {
        let bits = srr1_bits(43, 47);
        format!("PROGRAM_INTERRUPT {reason} at SRR0={srr0} SRR1[43:47]={bits}")
    };
    match class {
        C::InstructionStorage => {
            let bits = srr1_bits(33, 36);
            format!("INSTRUCTION_STORAGE at SRR0={srr0} SRR1[33:36]={bits}")
        }
        C::InstructionSegment => format!("INSTRUCTION_SEGMENT at SRR0={srr0}"),
        C::DataStorage => {
            format!("DATA_STORAGE {access} at SRR0={srr0} DAR={dar} DSISR={dsisr}")
        }
        C::DataSegment => {
            format!("DATA_SEGMENT {access} at SRR0={srr0} DAR={dar} DSISR={dsisr}")
        }
        C::Alignment => format!("ALIGNMENT {access} at SRR0={srr0} DAR={dar}"),
        C::ProgramFpEnabled => program("fp-enabled"),
        C::ProgramIllegal => format!("{} (raw={raw})", program("illegal")),
        C::ProgramPrivileged => format!("{} (raw={raw})", program("privileged")),
        C::ProgramTrap => program(&format!("trap TO=0x{:02x}", code & 0xFF)),
        C::FpUnavailable => format!("FP_UNAVAILABLE at SRR0={srr0}"),
        C::VxuUnavailable => format!("VXU_UNAVAILABLE at SRR0={srr0}"),
        C::SystemCall => format!(
            "SYSTEM_CALL LEV={} (hypervisor call) at PC={pc} SRR0={srr0}",
            code & 0xFFFF
        ),
        C::Unimplemented => {
            let kind = cellgov_ppu::unimplemented_kind(code).map_or_else(
                || format!("<unknown kind 0x{:04x}>", code & 0x0000_FFFF),
                |kind| <&'static str>::from(kind).to_string(),
            );
            format!("UNIMPLEMENTED_INSN {kind} at PC={pc} (raw={raw})")
        }
        C::DebugBreak => format!("DEBUG_BREAK at PC={pc}"),
        C::Internal => format!("INTERNAL_STOP at PC={pc} (raw={raw})"),
    }
}

/// `unit` is the faulting unit; every byte the report shows is read
/// through that unit's own address space.
pub(crate) fn format_fault(
    rt: &Runtime,
    unit: cellgov_event::UnitId,
    result: &cellgov_exec::ExecutionStepResult,
    fault: &cellgov_effects::FaultKind,
    steps: usize,
    pc_ring: &PcRing,
    dump_mem_fault_ranges: &[(u64, u64)],
) -> String {
    let mem = unit_memory(rt, unit);
    let pc = result.local_diagnostics.pc;
    let pc_str = pc
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    let ea_str = result
        .local_diagnostics
        .faulting_ea
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    let raw_str = match pc {
        None => "<no-pc>".to_string(),
        Some(a) => match fetch_raw_at(mem, a) {
            Some(w) => format!("0x{w:08x}"),
            None => "<unmapped>".to_string(),
        },
    };
    let detail = match fault {
        cellgov_effects::FaultKind::Guest(code) => {
            match cellgov_ppu::PpuFaultClass::of_code(*code) {
                Some(class) => ppu_fault_line(class, *code, &result.local_diagnostics, &raw_str),
                // The SPU's classes land here: their constants are private
                // to `cellgov_spu`, which names them. The detail half holds
                // at most the low 16 bits of an address; `faulting_ea`
                // holds it whole.
                None => {
                    let name = cellgov_spu::describe_guest_fault(*code)
                        .map(|name| format!(" {name}"))
                        .unwrap_or_default();
                    match result.local_diagnostics.faulting_ea {
                        Some(_) => {
                            format!("Guest(0x{code:08x}){name} at PC={pc_str} (ea={ea_str})")
                        }
                        None => format!("Guest(0x{code:08x}){name} at PC={pc_str}"),
                    }
                }
            }
        }
        _ => format!("Validation at PC={pc_str}"),
    };
    let mut out = format!("FAULT at step {steps}: {detail}");

    if let Some(regs) = &result.local_diagnostics.fault_regs {
        append_register_block(&mut out, mem, regs);
    }

    append_explicit_mem_dump(&mut out, mem, dump_mem_fault_ranges);
    append_pc_ring_with_decode(&mut out, rt, pc_ring);

    out
}

/// A unit that stepped or faulted is tagged to a live space, so a
/// miss is runtime-state corruption.
pub(crate) fn unit_memory(rt: &Runtime, unit: cellgov_event::UnitId) -> &GuestMemory {
    let space = rt.unit_space(unit);
    rt.space_memory(space).unwrap_or_else(|e| {
        panic!(
            "unit {} is tagged to address space {} which does not exist: {e}",
            unit.raw(),
            space.raw()
        )
    })
}

/// Append the registers section of a fault report: the GPR grid, the
/// special registers, the pointer dump and the stack walk.
fn append_register_block(
    out: &mut String,
    mem: &GuestMemory,
    regs: &cellgov_exec::FaultRegisterDump,
) {
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
    append_register_pointer_dump(out, mem, regs);
    append_stack_walk(out, mem, regs);
}

/// Dump 64 bytes at any GPR/LR/CTR value >= 0x1000 (plausible pointer).
pub(crate) fn append_register_pointer_dump(
    out: &mut String,
    mem: &GuestMemory,
    regs: &cellgov_exec::FaultRegisterDump,
) {
    let gprs = regs
        .gprs
        .iter()
        .enumerate()
        .map(|(i, &val)| (format!("r{i}"), val));
    let specials =
        [("LR", regs.lr), ("CTR", regs.ctr)].map(|(label, val)| (label.to_string(), val));
    let mut pointers = gprs
        .chain(specials)
        .filter(|&(_, val)| val >= 0x1000)
        .peekable();
    if pointers.peek().is_none() {
        return;
    }
    out.push_str("\n  register pointers:");
    for (label, val) in pointers {
        append_register_pointer_line(out, mem, &label, val);
    }
}

fn append_register_pointer_line(out: &mut String, mem: &GuestMemory, label: &str, val: u64) {
    let Some(range) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(val), 64) else {
        out.push_str(&format!(
            "\n    [{label}=0x{val:016x}]: <invalid address range>"
        ));
        return;
    };
    let Some(slice) = mem.read(range) else {
        out.push_str(&format!("\n    [{label}=0x{val:016x}]: <unreadable>"));
        return;
    };
    let region = region_label_at(mem, val, 64);
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

/// Render `--dump-mem-fault` ranges as hex+ASCII; a range with an unmapped tail emits the readable prefix.
pub(crate) fn append_explicit_mem_dump(out: &mut String, mem: &GuestMemory, ranges: &[(u64, u64)]) {
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
        let region = region_label_at(mem, addr, len);
        match mem.read(range) {
            Some(slice) => {
                out.push_str(&format!(
                    "\n    mem[0x{addr:016x} ({region}), {len} bytes]:"
                ));
                append_hex_ascii_block(out, addr, &slice);
            }
            None => match longest_readable_prefix(mem, addr, len) {
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

pub(crate) fn format_commit_fault(
    rt: &Runtime,
    err: &CommitError,
    steps: usize,
    source: cellgov_event::UnitId,
    pc_ring: &PcRing,
) -> String {
    let mut out = format!("COMMIT_FAULT at step {steps}: {err:?}");
    let mem = unit_memory(rt, source);
    // PC-side `FAULT` populates `result.local_diagnostics.fault_regs`
    // before commit runs; the commit path has no equivalent, so dump
    // the source unit's post-step state via `RegisteredUnit::register_dump`.
    if let Some(regs) = rt.registry().get(source).and_then(|u| u.register_dump()) {
        out.push_str(&format!("\n  source unit: {}", source.raw()));
        append_register_block(&mut out, mem, &regs);
    }
    append_pc_ring_with_decode(&mut out, rt, pc_ring);
    out
}

/// Walks `rt.registry()` rather than `Lv2Host::ppu_threads()` so SPU units
/// blocked on mailbox-receive or DMA wait surface alongside PPU threads.
pub(crate) fn format_deadlock(rt: &Runtime, steps: usize, pc_ring: &PcRing) -> String {
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
    append_pc_ring_terse(&mut out, pc_ring);
    out
}

#[cfg(test)]
#[path = "tests/fault_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fault_fallback_tests.rs"]
mod fallback_arm_tests;

#[cfg(test)]
#[path = "tests/fault_class_tests.rs"]
mod class_line_tests;
