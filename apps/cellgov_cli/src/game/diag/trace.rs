use cellgov_core::Runtime;

use super::{ascii_safe_preview, fetch_raw_at, format_hle_idx, longest_readable_prefix};

pub(in crate::game) fn print_trace_line(
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
