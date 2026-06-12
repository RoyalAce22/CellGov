//! `dump-prx-imports` subcommand: parse any PRX/SPRX's import table
//! and print it. Handles both raw `.prx` (plaintext ELF) and `.sprx`
//! (SCE-wrapped) inputs. `--at` matches `ImportedFunction::stub_addr`
//! by exact equality against the file-relative vaddr at parse time
//! (this tool does not run relocations).
//!
//! `--save-elf <path>` writes the decrypted plaintext ELF (the same
//! bytes the parser consumes) to `path`. The output is byte-identical
//! to RPCS3's `unself` for the same input. Useful when the firmware
//! PRX of interest (`libsysutil_avconf_ext`, `libsre`, `libfiber`)
//! isn't pre-staged under `tools/rpcs3/dev_flash_decrypted/` and you
//! need an ELF to pass to `cellgov_cli disasm`.

const NAME_COLUMN_WIDTH: usize = 49;

fn fit_name_column(name: &str) -> String {
    if name.chars().count() <= NAME_COLUMN_WIDTH {
        name.to_string()
    } else {
        let head: String = name.chars().take(NAME_COLUMN_WIDTH - 3).collect();
        format!("{head}...")
    }
}

#[derive(Debug)]
struct Args {
    path: std::path::PathBuf,
    filter_addr: Option<u32>,
    filter_module: Option<String>,
    save_elf: Option<std::path::PathBuf>,
}

/// Parse argv into [`Args`]; `Err` is a usage-format string ready
/// for `die`.
fn try_parse_args(args: &[String]) -> Result<Args, String> {
    debug_assert!(
        args.len() >= 2 && args[1] == "dump-prx-imports",
        "dump_prx_imports::run was dispatched with unexpected argv head: {args:?}",
    );

    let mut path: Option<std::path::PathBuf> = None;
    let mut filter_addr: Option<u32> = None;
    let mut filter_module: Option<String> = None;
    let mut save_elf: Option<std::path::PathBuf> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--at" => {
                if filter_addr.is_some() {
                    return Err("dump-prx-imports: --at specified more than once".to_string());
                }
                let v = args.get(i + 1).ok_or_else(|| {
                    "--at requires a hex value (e.g. --at 0x009bff10)".to_string()
                })?;
                let stripped = v
                    .strip_prefix("0x")
                    .or_else(|| v.strip_prefix("0X"))
                    .unwrap_or(v);
                let parsed = u32::from_str_radix(stripped, 16)
                    .map_err(|e| format!("--at {v:?}: not a hex u32 ({e})"))?;
                filter_addr = Some(parsed);
                i += 2;
            }
            "--module" => {
                if filter_module.is_some() {
                    return Err("dump-prx-imports: --module specified more than once".to_string());
                }
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--module requires a name".to_string())?;
                filter_module = Some(v.clone());
                i += 2;
            }
            "--save-elf" => {
                if save_elf.is_some() {
                    return Err("dump-prx-imports: --save-elf specified more than once".to_string());
                }
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--save-elf requires an output path".to_string())?;
                save_elf = Some(std::path::PathBuf::from(v));
                i += 2;
            }
            other if other.starts_with("--") => {
                return Err(format!("dump-prx-imports: unknown flag {other}"));
            }
            _ => {
                if path.is_some() {
                    return Err(
                        "dump-prx-imports: only one positional path argument is accepted"
                            .to_string(),
                    );
                }
                path = Some(std::path::PathBuf::from(&args[i]));
                i += 1;
            }
        }
    }

    let path = path.ok_or_else(|| {
        "usage: cellgov_cli dump-prx-imports <path-to-prx-or-sprx> \
         [--at 0xADDR] [--module NAME] [--save-elf <path>]"
            .to_string()
    })?;

    Ok(Args {
        path,
        filter_addr,
        filter_module,
        save_elf,
    })
}

use cellgov_ps3_abi::elf::ELF_MAGIC;
use cellgov_ps3_abi::sce::SCE_MAGIC;

#[derive(Debug, PartialEq, Eq)]
enum SourceKind {
    Elf,
    SceWrapped,
}

impl SourceKind {
    fn as_label(&self) -> &'static str {
        match self {
            Self::Elf => "ELF",
            Self::SceWrapped => "SCE -> ELF",
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum LoadError {
    #[error("PRX too small for header (got {len} bytes)")]
    TooSmall { len: usize },
    #[error("PRX bad magic: got {:02x} {:02x} {:02x} {:02x}", magic[0], magic[1], magic[2], magic[3])]
    BadMagic { magic: [u8; 4] },
}

/// Read `path` and return its plaintext ELF bytes plus the source
/// kind. Auto-detects SCE wrappers by magic and decrypts them.
fn load_elf_bytes(path: &std::path::Path) -> (Vec<u8>, SourceKind) {
    let raw = std::fs::read(path).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("dump-prx-imports: read {}: {e}", path.display()))
    });
    match classify_source(&raw) {
        Ok(SourceKind::Elf) => (raw, SourceKind::Elf),
        Ok(SourceKind::SceWrapped) => {
            let elf = cellgov_firmware::sce::decrypt_self_to_elf(&raw).unwrap_or_else(|e| {
                crate::cli::exit::die(&format!(
                    "dump-prx-imports: SCE decrypt of {} failed: {e}",
                    path.display()
                ))
            });
            (elf, SourceKind::SceWrapped)
        }
        Err(LoadError::TooSmall { len }) => crate::cli::exit::die(&format!(
            "dump-prx-imports: {} is {len} byte(s); needs at least {} for an ELF64 header",
            path.display(),
            cellgov_ps3_abi::elf::ELF_HEADER_SIZE,
        )),
        Err(LoadError::BadMagic { magic }) => crate::cli::exit::die(&format!(
            "dump-prx-imports: {} has unrecognized magic 0x{:02x}{:02x}{:02x}{:02x} \
             (expected ELF or SCE)",
            path.display(),
            magic[0],
            magic[1],
            magic[2],
            magic[3]
        )),
    }
}

/// Classify `raw`'s first 4 bytes as ELF or SCE magic.
fn classify_source(raw: &[u8]) -> Result<SourceKind, LoadError> {
    if raw.len() < cellgov_ps3_abi::elf::ELF_HEADER_SIZE {
        return Err(LoadError::TooSmall { len: raw.len() });
    }
    let magic: [u8; 4] = raw[0..4].try_into().expect("4-byte prefix");
    if magic == ELF_MAGIC {
        return Ok(SourceKind::Elf);
    }
    if magic == SCE_MAGIC {
        return Ok(SourceKind::SceWrapped);
    }
    Err(LoadError::BadMagic { magic })
}

pub(crate) fn run(args: &[String]) {
    let parsed = try_parse_args(args).unwrap_or_else(|msg| crate::cli::exit::die(&msg));
    let (elf_bytes, source_kind) = load_elf_bytes(&parsed.path);

    if let Some(out) = &parsed.save_elf {
        std::fs::write(out, &elf_bytes).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!(
                "dump-prx-imports: --save-elf write {}: {e}",
                out.display()
            ))
        });
        println!(
            "dump-prx-imports: wrote {} byte(s) of plaintext ELF to {}",
            elf_bytes.len(),
            out.display()
        );
    }

    // Best-effort: game EBOOTs / partial fixtures parse imports
    // but not the full PRX; degrade by omitting module identity.
    let sprx_parsed = cellgov_ppu::sprx::parse_prx(&elf_bytes).ok();

    let modules = cellgov_ppu::prx::parse_imports(&elf_bytes).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("dump-prx-imports: parse_imports failed: {e}"))
    });

    let total_funcs: usize = modules.iter().map(|m| m.functions.len()).sum();
    let path_str = parsed.path.to_string_lossy().replace('\\', "/");

    println!("# PRX Import Inventory");
    println!();
    println!("- File: `{path_str}`");
    println!("- Source: {}", source_kind.as_label());
    if let Some(p) = &sprx_parsed {
        println!("- Module name: `{}`", p.name);
        let ns_names: Vec<&str> = p.exports.iter().map(|e| e.name.as_str()).collect();
        if ns_names.is_empty() {
            println!("- Exports under: <none>");
        } else {
            println!("- Exports under: {}", ns_names.join(", "));
        }
    }
    println!("- Modules imported: {}", modules.len());
    println!("- Functions imported: {total_funcs}");
    if let Some(a) = parsed.filter_addr {
        println!("- Filter: --at 0x{a:08x}");
    }
    if let Some(m) = &parsed.filter_module {
        println!("- Filter: --module {m}");
    }
    println!();

    let mut matched = 0usize;
    let mut filter_module_seen = parsed.filter_module.is_none();
    let mut empty_modules: Vec<String> = Vec::new();

    for module in &modules {
        if let Some(want) = &parsed.filter_module {
            if module.name != *want {
                continue;
            }
            filter_module_seen = true;
        }
        if module.functions.is_empty() {
            empty_modules.push(module.name.clone());
            continue;
        }

        let matches: Vec<_> = module
            .functions
            .iter()
            .filter(|f| parsed.filter_addr.is_none_or(|a| f.stub_addr == a))
            .collect();
        if matches.is_empty() {
            continue;
        }

        println!(
            "## {} ({} function{})",
            module.name,
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        );
        println!();
        println!(
            "| NID        | Stub addr   | Name                                              | Class           |"
        );
        println!(
            "|------------|-------------|---------------------------------------------------|-----------------|"
        );
        for f in matches {
            let name = cellgov_ps3_abi::nid::lookup(f.nid)
                .map(|(_m, n)| n)
                .unwrap_or("<unknown>");
            let class_cell = cellgov_ps3_abi::nid::stub_classification(f.nid).as_str();
            println!(
                "| 0x{:08x} | 0x{:08x}  | {:<width$} | {:<15} |",
                f.nid,
                f.stub_addr,
                fit_name_column(name),
                class_cell,
                width = NAME_COLUMN_WIDTH,
            );
            matched += 1;
        }
        println!();
    }

    if parsed.filter_addr.is_some() || parsed.filter_module.is_some() {
        println!("Matched {matched} import(s).");
    }

    if let Some(want) = &parsed.filter_module {
        if !filter_module_seen {
            eprintln!(
                "dump-prx-imports: --module {want:?} not found in {} imported module(s)",
                modules.len()
            );
        } else if empty_modules.iter().any(|n| n == want) {
            eprintln!("dump-prx-imports: --module {want:?} declares no functions");
        }
    }

    if let Some(target) = parsed.filter_addr {
        if matched == 0 {
            let scope: Vec<&cellgov_ppu::prx::ImportedModule> = match &parsed.filter_module {
                Some(want) => modules.iter().filter(|m| m.name == *want).collect(),
                None => modules.iter().collect(),
            };
            if let Some(hint) = nearest_stub_hint(&scope, target) {
                eprintln!("dump-prx-imports: {hint}");
            }
        }
    }

    // Skip the empty-modules trailer on filtered runs; the count
    // would misleadingly read as file-wide.
    let unfiltered = parsed.filter_addr.is_none() && parsed.filter_module.is_none();
    if unfiltered && !empty_modules.is_empty() {
        eprintln!(
            "dump-prx-imports: {} module(s) declared in the import table have no functions; \
             omitted from the listing:",
            empty_modules.len()
        );
        for name in &empty_modules {
            eprintln!("  {name}");
        }
    }
}

/// Build a single-line hint pointing at the closest declared
/// `stub_addr` to `target`. Useful when a user types a fault PC
/// mid-stub and gets no exact match.
fn nearest_stub_hint(modules: &[&cellgov_ppu::prx::ImportedModule], target: u32) -> Option<String> {
    let mut best: Option<(u32, &str, u32)> = None; // (distance, module, stub_addr)
    for m in modules {
        for f in &m.functions {
            let dist = f.stub_addr.abs_diff(target);
            if best.is_none_or(|(d, _, _)| dist < d) {
                best = Some((dist, m.name.as_str(), f.stub_addr));
            }
        }
    }
    best.map(|(dist, module, stub_addr)| {
        format!(
            "no exact match for 0x{target:08x}; nearest declared stub is \
             {module}::0x{stub_addr:08x} (distance {dist} byte(s))",
        )
    })
}

#[cfg(test)]
#[path = "tests/dump_prx_imports_tests.rs"]
mod tests;
