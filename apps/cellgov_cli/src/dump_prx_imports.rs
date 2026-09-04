//! `dev prx-imports`: parse any PRX/SPRX's import table
//! and print it. Handles both raw `.prx` (plaintext ELF) and `.sprx`
//! (SCE-wrapped) inputs. `--at` matches `ImportedFunction::stub_addr`
//! by exact equality against the file-relative vaddr at parse time
//! (this tool does not run relocations).
//!
//! `--save-elf <path>` writes the decrypted plaintext ELF to `path`:
//! the same bytes the parser consumed for the printed table.

use crate::cli::parse::PrxImportsArgs;

const NAME_COLUMN_WIDTH: usize = 49;

fn fit_name_column(name: &str) -> String {
    if name.chars().count() <= NAME_COLUMN_WIDTH {
        name.to_string()
    } else {
        let head: String = name.chars().take(NAME_COLUMN_WIDTH - 3).collect();
        format!("{head}...")
    }
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
/// kind. Auto-detects SCE wrappers by magic and decrypts them; NPDRM
/// SELFs resolve their RAP from `vfs_root`'s exdata directory.
fn load_elf_bytes(path: &std::path::Path, vfs_root: &std::path::Path) -> (Vec<u8>, SourceKind) {
    let raw = std::fs::read(path).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("prx-imports: read {}: {e}", path.display()))
    });
    match classify_source(&raw) {
        Ok(SourceKind::Elf) => (raw, SourceKind::Elf),
        Ok(SourceKind::SceWrapped) => {
            let elf = crate::cli::exit::decrypt_ppu_self_or_die(
                &raw,
                &path.display().to_string(),
                vfs_root,
            );
            (elf, SourceKind::SceWrapped)
        }
        Err(LoadError::TooSmall { len }) => crate::cli::exit::die(&format!(
            "prx-imports: {} is {len} byte(s); needs at least {} for an ELF64 header",
            path.display(),
            cellgov_ps3_abi::elf::ELF_HEADER_SIZE,
        )),
        Err(LoadError::BadMagic { magic }) => crate::cli::exit::die(&format!(
            "prx-imports: {} has unrecognized magic 0x{:02x}{:02x}{:02x}{:02x} \
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

/// The module-identity block for the listing header, or the reason
/// there is none.
///
/// `Ok(None)` is the title-executable case: `e_type` is ET_EXEC, so
/// no `sys_prx_module_info_t` exists and none is expected. A PPU
/// object on this platform carries one of exactly two ELF types, both
/// in [`cellgov_ps3_abi::elf`]. `ET_EXEC` names a title executable.
/// The PS3 relocatable-module type names every firmware module under
/// `dev_flash/sys/external`. Every other `e_type` is a structural
/// anomaly in a file whose import table the caller prints as
/// authoritative, so this returns the refusal for the caller to name.
fn module_identity(
    elf_bytes: &[u8],
) -> Result<Option<cellgov_ppu::sprx::ParsedPrx>, cellgov_ppu::sprx::PrxParseError> {
    match cellgov_ppu::sprx::parse_prx(elf_bytes) {
        Ok(p) => Ok(Some(p)),
        Err(cellgov_ppu::sprx::PrxParseError::NotPrx(t)) if t == cellgov_ps3_abi::elf::ET_EXEC => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn run(parsed: &PrxImportsArgs, vfs_flag: Option<&std::path::Path>) {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag);
    let (elf_bytes, source_kind) = load_elf_bytes(&parsed.path, &vfs_root);

    if let Some(out) = &parsed.save_elf {
        std::fs::write(out, &elf_bytes).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!(
                "prx-imports: --save-elf write {}: {e}",
                out.display()
            ))
        });
        println!(
            "prx-imports: wrote {} byte(s) of plaintext ELF to {}",
            elf_bytes.len(),
            out.display()
        );
    }

    let sprx_parsed = match module_identity(&elf_bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "prx-imports: {}: PRX module info: {e}; \
                 module name, export namespaces, segment geometry, module TOC \
                 and entry-point OPDs omitted from the listing",
                parsed.path.display(),
            );
            None
        }
    };

    let modules = cellgov_ppu::prx::parse_imports(&elf_bytes).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("prx-imports: parse_imports failed: {e}"))
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
        // Every vaddr below is unrelocated PRX-space, the frame the
        // import table prints stub addresses in. `sprx::load_prx` adds
        // the load base, so none of these matches a fault PC from a
        // booted run.
        println!(
            "- Text segment: vaddr 0x{:08x} (unrelocated), 0x{:x} byte(s)",
            p.text.vaddr, p.text.memsz
        );
        println!(
            "- Data segment: vaddr 0x{:08x} (unrelocated), 0x{:x} byte(s)",
            p.data.vaddr, p.data.memsz
        );
        println!("- Module TOC: 0x{:08x} (unrelocated)", p.toc);
        println!("- module_start: {}", describe_opd(p.module_start));
        println!("- module_stop: {}", describe_opd(p.module_stop));
    }
    println!("- Modules imported: {}", modules.len());
    println!("- Functions imported: {total_funcs}");
    if let Some(a) = parsed.at {
        println!("- Filter: --at 0x{a:08x}");
    }
    if let Some(m) = &parsed.module {
        println!("- Filter: --module {m}");
    }
    println!();

    let mut matched = 0usize;
    let mut filter_module_seen = parsed.module.is_none();
    let mut empty_modules: Vec<String> = Vec::new();

    for module in &modules {
        if let Some(want) = &parsed.module {
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
            .filter(|f| parsed.at.is_none_or(|a| f.stub_addr == a))
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

    if parsed.at.is_some() || parsed.module.is_some() {
        println!("Matched {matched} import(s).");
    }

    if let Some(want) = &parsed.module {
        if !filter_module_seen {
            eprintln!(
                "prx-imports: --module {want:?} not found in {} imported module(s)",
                modules.len()
            );
        } else if empty_modules.iter().any(|n| n == want) {
            eprintln!("prx-imports: --module {want:?} declares no functions");
        }
    }

    if let Some(target) = parsed.at {
        if matched == 0 {
            let scope: Vec<&cellgov_ppu::prx::ImportedModule> = match &parsed.module {
                Some(want) => modules.iter().filter(|m| m.name == *want).collect(),
                None => modules.iter().collect(),
            };
            if let Some(hint) = nearest_stub_hint(&scope, target) {
                eprintln!("prx-imports: {hint}");
            }
        }
    }

    // Skip the empty-modules trailer on filtered runs; the count
    // would misleadingly read as file-wide.
    if parsed.is_unfiltered() && !empty_modules.is_empty() {
        eprintln!(
            "prx-imports: {} module(s) declared in the import table have no functions; \
             omitted from the listing:",
            empty_modules.len()
        );
        for name in &empty_modules {
            eprintln!("  {name}");
        }
    }
}

/// Render an entry-point OPD, in the listing's unrelocated PRX-space vaddrs.
///
/// The parser returns `None` for two cases:
///
/// - the module exports no such entry point;
/// - the located OPD pairs its entry with TOC 0, which the parser
///   refuses as a corrupt descriptor.
fn describe_opd(opd: Option<cellgov_ppu::sprx::PrxOpd>) -> String {
    match opd {
        Some(o) => format!(
            "OPD 0x{:08x} -> code 0x{:08x}, toc 0x{:08x}",
            o.opd_vaddr, o.code, o.toc
        ),
        None => "<none> (not exported, or its OPD carries toc 0)".to_string(),
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

#[cfg(test)]
#[path = "tests/dump_prx_imports_opd_tests.rs"]
mod opd_tests;
