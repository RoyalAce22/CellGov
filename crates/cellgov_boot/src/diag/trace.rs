//! One line per retired step, for `boot run --trace`.

use cellgov_core::Runtime;

use super::{ascii_safe_preview, fetch_raw_at, format_hle_idx, longest_readable_prefix};
use crate::BootSink;

/// Report the step `unit` just retired: its PC, the raw word there, and
/// the syscall it dispatched.
pub(crate) fn report_trace_line(
    rt: &Runtime,
    unit: cellgov_event::UnitId,
    result: &cellgov_exec::ExecutionStepResult,
    steps: usize,
    sink: &dyn BootSink,
) {
    let mem = super::unit_memory(rt, unit);
    if let Some(pc) = result.local_diagnostics.pc {
        // Zero decodes as a valid PPC instruction; distinguish unmapped from a real zero word.
        let raw = fetch_raw_at(mem, pc)
            .map(|w| format!("0x{w:08x}"))
            .unwrap_or_else(|| "<unmapped>".to_string());
        sink.note(&format!(
            "[{steps:>4}] u{} PC=0x{pc:08x}  raw={raw}  yr={:?}",
            unit.raw(),
            result.yield_reason
        ));
    }
    if let Some(args) = &result.syscall_args {
        if args[0] >= 0x10000 {
            let idx = (args[0] - 0x10000) as u32;
            sink.note(&format!("       -> HLE #{idx}: {}", format_hle_idx(idx)));
        } else if args[0] == 403 {
            let buf = args[2];
            let len = args[3];
            let full = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), len)
                .and_then(|r| mem.read(r));
            match full {
                Some(slice) => {
                    let text = String::from_utf8_lossy(slice);
                    let terminator = if text.ends_with('\n') { "" } else { "\n" };
                    sink.guest_text(&format!("       -> tty: {text}{terminator}"));
                }
                None => match longest_readable_prefix(mem, buf, len) {
                    Some((n, bytes)) => {
                        let text = ascii_safe_preview(&bytes);
                        sink.note(&format!("       -> tty (partial {n}/{len}): {text}"));
                    }
                    None => sink.note(&format!("       -> LV2 tty_write (oob, 0/{len} readable)")),
                },
            }
        } else {
            sink.note(&format!("       -> LV2 syscall {}", args[0]));
        }
    }
}
