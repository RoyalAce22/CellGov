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

use crate::cli::exit::die;

/// Process exit code when at least one decoded word was an unsupported
/// encoding. Distinct from 1 (fatal CLI error via `die`) so wrappers
/// can tell apart "bad inputs" from "decoded the bytes; some weren't
/// instructions".
const DECODE_ERROR_EXIT_CODE: i32 = 2;

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
            Err(e) => die(&e.message()),
        };

    // Contract: exit code DECODE_ERROR_EXIT_CODE iff at least one
    // word failed to decode. A boundary marker (BSS / past-end) on
    // its own is not an error.
    if stats.decode_errors > 0 {
        std::process::exit(DECODE_ERROR_EXIT_CODE);
    }
}
