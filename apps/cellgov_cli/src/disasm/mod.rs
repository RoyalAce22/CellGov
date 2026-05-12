//! Read-only PowerPC disassembler that delegates decoding to
//! `cellgov_ppu::decode::decode`.
//!
//! Used to investigate guest behavior at specific addresses without
//! booting the title. Output format: `addr  raw  decoded` per
//! instruction. The instruction stream goes to stdout; structural
//! diagnostics ("past segment end", overlap warnings, data heuristic)
//! go to stderr so a downstream tool can pipe stdout cleanly.

mod args;
mod elf;
mod stream;

#[cfg(test)]
mod test_support;

use std::io::Write;

use crate::cli::exit::die;
use stream::StreamError;

/// Process exit code when at least one decoded word was an unsupported
/// encoding. Distinct from 1 (fatal CLI error via `die`) so wrappers
/// can tell apart "bad inputs" from "decoded the bytes; some weren't
/// instructions".
const DECODE_ERROR_EXIT_CODE: i32 = 2;

/// Process exit code for a stdout closed by a downstream pipe reader
/// (`| head`, `| less` quit early). Matches coreutils' 128 + SIGPIPE
/// convention so shell pipelines can distinguish "consumer left" from
/// any of our other exit modes.
const BROKEN_PIPE_EXIT_CODE: i32 = 141;

pub(crate) fn run(args: &[String]) {
    let parsed = args::parse_args(args).unwrap_or_else(|e| die(&e.message()));
    let elf_bytes = std::fs::read(parsed.elf_path)
        .unwrap_or_else(|e| die(&format!("read elf {}: {e}", parsed.elf_path)));
    let segments = elf::parse_pt_loads(&elf_bytes).unwrap_or_else(|e| die(&e.message()));

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stats =
        match stream::disassemble(&elf_bytes, &segments, parsed.vaddr, parsed.count, &mut out) {
            Ok(s) => s,
            Err(StreamError::BadVaddr(e)) => die(&e.message()),
            Err(StreamError::Io(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                std::process::exit(BROKEN_PIPE_EXIT_CODE);
            }
            Err(StreamError::Io(e)) => die(&format!("disasm: stdout write: {e}")),
        };

    // process::exit skips destructors, so the StdoutLock's drop-flush
    // never runs. Under `| less` or `> file`, the buffered tail of the
    // stream would be lost. Flush explicitly before any exit below.
    if let Err(e) = out.flush() {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(BROKEN_PIPE_EXIT_CODE);
        }
        die(&format!("disasm: stdout flush: {e}"));
    }

    // Contract: exit code DECODE_ERROR_EXIT_CODE iff at least one
    // word failed to decode. A boundary marker (BSS / past-end) on
    // its own is not an error.
    if stats.decode_errors > 0 {
        std::process::exit(DECODE_ERROR_EXIT_CODE);
    }
}
