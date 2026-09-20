//! `dev disasm` entry point: check the address, read the ELF, drive
//! the streaming disassembler, and translate stream errors into the
//! documented exit-code contract.

use std::io::Write;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::DisasmArgs;
use crate::disasm::stream::StreamError;
use crate::disasm::{args, elf, stream};

/// Process exit code when at least one decoded word was an unsupported
/// encoding. A wrapper can then tell "bad inputs" from "decoded the
/// bytes; some weren't instructions".
const DECODE_ERROR_EXIT_CODE: i32 = exit_codes::command_specific(20);

pub(crate) fn run(
    parsed: &DisasmArgs,
    vfs_flag: Option<&std::path::Path>,
) -> Result<CommandExitCode, CommandError> {
    args::check_alignment(parsed.vaddr)
        .map_err(|error| CommandError::status(exit_codes::USAGE, error.to_string()))?;
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let raw = crate::cli::self_load::load_file(&parsed.elf_path)?;
    // Transparently decrypt an SCE/SELF wrapper (including NPDRM
    // EBOOTs); plaintext ELF input passes through unchanged.
    let elf_bytes = crate::cli::self_load::decrypt_ppu_self(&raw, &parsed.elf_path, &vfs_root)?;
    let segments =
        elf::parse_pt_loads(&elf_bytes).map_err(|error| CommandError::failed(error.message()))?;

    let symbols = if parsed.symbolize {
        let mut map = cellgov_ppu::funcmap::build(&elf_bytes)
            .map_err(|error| CommandError::failed(format!("disasm --symbolize: {error}")))?;
        crate::funcs::resolve_nids(&mut map);
        Some(map)
    } else {
        None
    };

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
        Err(StreamError::BadVaddr(error)) => {
            return Err(CommandError::failed(error.message()));
        }
        Err(StreamError::Io(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            return Ok(CommandExitCode::new(exit_codes::BROKEN_PIPE));
        }
        Err(StreamError::Io(error)) => {
            return Err(CommandError::failed(format!(
                "disasm: stdout write: {error}"
            )));
        }
    };

    if let Err(e) = out.flush() {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(CommandExitCode::new(exit_codes::BROKEN_PIPE));
        }
        return Err(CommandError::failed(format!("disasm: stdout flush: {e}")));
    }

    // Contract: exit code DECODE_ERROR_EXIT_CODE iff at least one
    // word failed to decode. A boundary marker (BSS / past-end) on
    // its own is not an error.
    if stats.decode_errors > 0 {
        return Ok(CommandExitCode::new(DECODE_ERROR_EXIT_CODE));
    }
    Ok(CommandExitCode::SUCCESS)
}
