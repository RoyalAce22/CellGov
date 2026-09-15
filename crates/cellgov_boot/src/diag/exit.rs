//! Terminal-state diagnostics: a clean process exit, and the
//! max-steps report with every unit's state.

use cellgov_core::Runtime;
use cellgov_lv2::PpuThreadState;

use crate::step_loop::{block_reason_label, PcRing, SyscallRing};

use super::ascii_safe_preview;
use super::rings::{append_pc_ring_terse, append_syscall_ring};

/// The last guest TTY write a run captured.
#[derive(Debug, Clone)]
pub struct TtyCapture {
    /// The fd the guest named, narrowed to a sentinel when oversized.
    pub fd: u32,
    /// The bytes, at full fidelity.
    pub raw_bytes: Vec<u8>,
    /// PC of the `sys_tty_write` call.
    pub call_pc: u64,
}

/// The last process-exit syscall a run saw.
#[derive(Debug, Clone, Copy)]
pub struct ProcessExitInfo {
    /// The status the guest passed.
    pub code: u32,
    /// PC of the call.
    pub call_pc: u64,
}

/// The terminal report for a run that reached `sys_process_exit`.
pub fn format_process_exit(
    exit: &ProcessExitInfo,
    last_tty: Option<&TtyCapture>,
    steps: usize,
    pc_ring: &PcRing,
    syscall_ring: &SyscallRing,
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

    append_pc_ring_terse(&mut out, pc_ring);
    append_syscall_ring(&mut out, syscall_ring);
    out
}

/// The terminal report for a run that hit its step cap.
pub fn format_max_steps(
    rt: &Runtime,
    steps: usize,
    pc_ring: &PcRing,
    syscall_ring: &SyscallRing,
) -> String {
    let mut out = format!("MAX_STEPS after {} steps", steps);
    append_unit_state_summary(&mut out, rt);
    append_pc_ring_terse(&mut out, pc_ring);
    append_syscall_ring(&mut out, syscall_ring);
    out
}

/// One line per unit: id, effective status, LV2 PPU thread state if any.
pub(crate) fn append_unit_state_summary(out: &mut String, rt: &Runtime) {
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
