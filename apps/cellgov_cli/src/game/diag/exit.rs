use cellgov_core::Runtime;
use cellgov_lv2::PpuThreadState;

use crate::game::step_loop::{block_reason_label, RingCursor};
use crate::game::{PC_RING_SIZE, SYSCALL_RING_SIZE};

use super::ascii_safe_preview;
use super::rings::{append_pc_ring_terse, append_syscall_ring};

pub(in crate::game) struct TtyCapture {
    pub(in crate::game) fd: u32,
    pub(in crate::game) raw_bytes: Vec<u8>,
    pub(in crate::game) call_pc: u64,
}

pub(in crate::game) struct ProcessExitInfo {
    pub(in crate::game) code: u32,
    pub(in crate::game) call_pc: u64,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::game) fn format_process_exit(
    exit: &ProcessExitInfo,
    last_tty: Option<&TtyCapture>,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
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
    append_syscall_ring(&mut out, syscall_ring, syscall_cursor);
    out
}

pub(in crate::game) fn format_max_steps(
    rt: &Runtime,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_cursor: &RingCursor,
    syscall_ring: &[(u64, u64); SYSCALL_RING_SIZE],
    syscall_cursor: &RingCursor,
) -> String {
    let mut out = format!("MAX_STEPS after {} steps", steps);
    append_unit_state_summary(&mut out, rt);
    append_pc_ring_terse(&mut out, pc_ring, pc_cursor);
    append_syscall_ring(&mut out, syscall_ring, syscall_cursor);
    out
}

/// One line per unit: id, effective status, LV2 PPU thread state if any.
pub(in crate::game) fn append_unit_state_summary(out: &mut String, rt: &Runtime) {
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
