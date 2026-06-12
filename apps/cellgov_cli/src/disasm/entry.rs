//! `disasm` subcommand entry point: parse args, read the ELF, drive
//! the streaming disassembler, and translate stream errors into the
//! documented exit-code contract.

use std::io::Write;

use crate::cli::exit::die;
use crate::disasm::stream::StreamError;
use crate::disasm::{args, elf, stream};

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
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(args);
    let raw = crate::cli::exit::load_file_or_die(parsed.elf_path);
    // Transparently decrypt an SCE/SELF wrapper (including NPDRM
    // EBOOTs); plaintext ELF input passes through unchanged.
    let elf_bytes = crate::cli::exit::decrypt_ppu_self_or_die(&raw, parsed.elf_path, &vfs_root);
    let segments = elf::parse_pt_loads(&elf_bytes).unwrap_or_else(|e| die(&e.message()));

    let symbols = parsed.symbolize.then(|| {
        let mut map = cellgov_ppu::funcmap::build(&elf_bytes)
            .unwrap_or_else(|e| die(&format!("disasm --symbolize: {e}")));
        crate::funcs::resolve_nids(&mut map);
        map
    });

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stats = match stream::disassemble(
        &elf_bytes,
        &segments,
        parsed.vaddr,
        parsed.count,
        symbols.as_ref(),
        &mut out,
    ) {
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
