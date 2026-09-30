//! `dev spu-disasm`: the words of an SPU image as text.

use std::io::Write;
use std::path::Path;

use cellgov_install::self_image::is_sce_wrapped;
use cellgov_spu::disasm::{SpuWord, SpuWordClass};
use cellgov_spu::image::find_embedded_spu_elfs;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::SpuDisasmArgs;
use crate::cli::self_load::{decrypt_ppu_self, load_file};

/// Exit code when at least one word is no instruction the CBE runs.
const DECODE_ERROR_EXIT_CODE: i32 = exit_codes::command_specific(20);

/// A run of local-store bytes and the address its first byte loads at.
struct Span<'a> {
    lsa: u32,
    bytes: &'a [u8],
}

pub(crate) fn run(
    parsed: &SpuDisasmArgs,
    vfs_flag: Option<&Path>,
) -> Result<CommandExitCode, CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let file = load_file(&parsed.path)?;
    let data = if is_sce_wrapped(&file) {
        decrypt_ppu_self(&file, &parsed.path, &vfs_root)?
    } else {
        file
    };
    let images;
    let (spans, start) = if parsed.raw {
        let skip = parsed.skip as usize;
        let bytes = data.get(skip..).ok_or_else(|| {
            CommandError::failed(format!(
                "spu-disasm: --skip 0x{skip:x} is past the end of {} ({} bytes)",
                parsed.path,
                data.len()
            ))
        })?;
        (
            vec![Span {
                lsa: parsed.base,
                bytes,
            }],
            parsed.lsa.unwrap_or(parsed.base),
        )
    } else {
        images = find_embedded_spu_elfs(&data);
        let image = match (parsed.image, images.as_slice()) {
            (Some(n), _) => images.get(n).ok_or_else(|| {
                CommandError::failed(format!(
                    "spu-disasm: --image {n}, but {} holds {} SPU ELF image(s)",
                    parsed.path,
                    images.len()
                ))
            })?,
            (None, [only]) if only.offset == 0 => only,
            (None, []) => {
                return Err(CommandError::failed(format!(
                    "spu-disasm: {} holds no SPU ELF image; pass --raw to read it as a \
                     local-store image",
                    parsed.path
                )))
            }
            (None, _) => {
                for (n, image) in images.iter().enumerate() {
                    println!(
                        "image {n}: offset 0x{:x}  bytes 0x{:x}  entry 0x{:05x}  {} segment(s)",
                        image.offset,
                        image.elf.extent,
                        image.elf.entry,
                        image.elf.segments.len()
                    );
                }
                return Ok(CommandExitCode::SUCCESS);
            }
        };
        let host = image.bytes(&data);
        (
            image
                .elf
                .segments
                .iter()
                .map(|segment| Span {
                    lsa: segment.vaddr,
                    bytes: segment.bytes(host),
                })
                .collect(),
            parsed.lsa.unwrap_or(image.elf.entry),
        )
    };
    if !start.is_multiple_of(4) {
        return Err(CommandError::status(
            exit_codes::USAGE,
            format!("spu-disasm: --lsa 0x{start:x} is not 4-byte aligned; SPU instructions are aligned words"),
        ));
    }
    let Some(span) = spans
        .iter()
        .find(|s| (s.lsa..s.lsa.saturating_add(s.bytes.len() as u32)).contains(&start))
    else {
        let held: Vec<String> = spans
            .iter()
            .map(|s| format!("0x{:05x}+0x{:x}", s.lsa, s.bytes.len()))
            .collect();
        return Err(CommandError::failed(format!(
            "spu-disasm: local-store address 0x{start:05x} is in no loaded segment; the image \
             loads {}",
            held.join(", ")
        )));
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match write_words(&mut out, span, start, parsed.count) {
        Ok(invalid) => {
            if let Err(e) = out.flush() {
                return broken_pipe_or(e);
            }
            if invalid > 0 {
                return Ok(CommandExitCode::new(DECODE_ERROR_EXIT_CODE));
            }
            Ok(CommandExitCode::SUCCESS)
        }
        Err(e) => broken_pipe_or(e),
    }
}

/// Write `count` words of `span` from `start`, then a marker where the
/// span ends first. Returns the words that are no instruction the CBE
/// runs.
fn write_words<W: Write>(
    out: &mut W,
    span: &Span<'_>,
    start: u32,
    count: usize,
) -> std::io::Result<usize> {
    let mut invalid = 0;
    let first = (start - span.lsa) as usize;
    let mut words = span.bytes[first..].chunks_exact(4);
    for n in 0..count {
        let lsa = start + 4 * n as u32;
        let Some(bytes) = words.next() else {
            writeln!(out, "0x{lsa:05x}  --------  <past segment end>")?;
            break;
        };
        let word = SpuWord::of(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        let note = match word.class {
            SpuWordClass::Implemented => "",
            SpuWordClass::NotImplemented => "  ; not implemented",
            SpuWordClass::AbsentOnCbe => "  ; absent on the CBE",
            SpuWordClass::Unassigned => "",
        };
        if matches!(
            word.class,
            SpuWordClass::AbsentOnCbe | SpuWordClass::Unassigned
        ) {
            invalid += 1;
        }
        writeln!(out, "0x{lsa:05x}  {:08x}  {word}{note}", word.raw)?;
    }
    Ok(invalid)
}

fn broken_pipe_or(e: std::io::Error) -> Result<CommandExitCode, CommandError> {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        return Ok(CommandExitCode::new(exit_codes::BROKEN_PIPE));
    }
    Err(CommandError::failed(format!("spu-disasm: stdout: {e}")))
}

#[cfg(test)]
#[path = "tests/spu_disasm_tests.rs"]
mod tests;
