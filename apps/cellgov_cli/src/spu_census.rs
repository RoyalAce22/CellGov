//! `dev spu-census`: the SPU instruction words of installed titles,
//! counted by opcode-map row and decoder class.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::{TitleManifest, TitleRegistry};
use cellgov_install::self_image::is_sce_wrapped;
use cellgov_install::store::StoreInventory;
use cellgov_ps3_abi::format::elf::ELF_MAGIC;
use cellgov_spu::census::SpuCensus;
use cellgov_spu::disasm::SpuWordClass;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::{OutputFormat, SpuCensusArgs};
use crate::cli::self_load::decrypt_ppu_self;

/// The classes a census reports, in report order.
const CLASSES: [SpuWordClass; 4] = [
    SpuWordClass::Implemented,
    SpuWordClass::NotImplemented,
    SpuWordClass::AbsentOnCbe,
    SpuWordClass::Unassigned,
];

/// What the census read of one title.
struct TitleReport {
    name: String,
    /// Why nothing of the title was read, if nothing was.
    skipped: Option<String>,
    /// ELF and SELF files read.
    files: usize,
    /// SPU images first found in this title.
    images: usize,
    /// Words those images hold.
    words: u64,
    /// Files that could not be read, and why.
    unreadable: Vec<(PathBuf, String)>,
}

pub(crate) fn run(
    args: &SpuCensusArgs,
    vfs_flag: Option<&Path>,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let registry = TitleRegistry::scan_dir(&crate::cli::store::registry_dir())
        .map_err(|error| CommandError::failed(format!("spu-census: title registry: {error}")))?;
    let titles: Vec<&TitleManifest> = match &args.scope.title {
        Some(name) => vec![registry.by_short_name(name).ok_or_else(|| {
            CommandError::failed(format!(
                "spu-census: unknown title '{name}'. Known titles: {}",
                registry.known_names_csv()
            ))
        })?],
        None => registry.iter().collect(),
    };
    let inventory = StoreInventory::read(&crate::cli::keys::install_root_of(&vfs_root))
        .map_err(|error| CommandError::failed(format!("spu-census: store: {error}")))?;

    let mut seen = BTreeSet::new();
    let mut total = SpuCensus::new();
    let mut reports = Vec::new();
    for title in titles {
        reports.push(census_title(
            title, &inventory, &vfs_root, &mut seen, &mut total,
        ));
    }
    match format {
        OutputFormat::Human => print_human(&reports, &total, seen.len()),
        OutputFormat::Json => print_json(&reports, &total, seen.len())?,
    }
    Ok(CommandExitCode::SUCCESS)
}

/// Count the SPU images of one title that no earlier title held.
fn census_title(
    title: &TitleManifest,
    inventory: &StoreInventory,
    vfs_root: &Path,
    seen: &mut BTreeSet<u64>,
    total: &mut SpuCensus,
) -> TitleReport {
    let mut report = TitleReport {
        name: title.name().to_string(),
        skipped: None,
        files: 0,
        images: 0,
        words: 0,
        unreadable: Vec::new(),
    };
    if title.ships_in_firmware() {
        report.skipped = Some("ships inside the firmware".to_string());
        return report;
    }
    let dirs: Vec<PathBuf> = match inventory.title(&title.content_id) {
        Some(entry) => entry
            .base
            .iter()
            .map(|base| base.dir.clone())
            .chain(entry.updates.values().map(|update| update.dir.clone()))
            .collect(),
        None => title.eboot_dirs(vfs_root).unwrap_or_default(),
    };
    let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    if dirs.is_empty() {
        report.skipped = Some("not installed".to_string());
        return report;
    }
    let mut files = Vec::new();
    for dir in &dirs {
        walk(dir, &mut files, &mut report.unreadable);
    }
    files.sort();
    for path in files {
        let bytes = match read_image(&path, vfs_root) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(reason) => {
                report.unreadable.push((path, reason));
                continue;
            }
        };
        report.files += 1;
        let (images, words) = total.add_new_images(&bytes, seen);
        report.images += images;
        report.words += words;
    }
    report
}

/// Every regular file under `dir`, recursively.
fn walk(dir: &Path, files: &mut Vec<PathBuf>, unreadable: &mut Vec<(PathBuf, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            unreadable.push((dir.to_path_buf(), error.to_string()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, files, unreadable),
            Ok(kind) if kind.is_file() => files.push(path),
            _ => {}
        }
    }
}

/// The plaintext ELF bytes of `path`, or `None` for a file that is
/// neither an ELF nor an SCE container.
///
/// # Errors
///
/// Why the file could not be read or decrypted.
fn read_image(path: &Path, vfs_root: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .map_err(|error| error.to_string())?;
    let head = &head[..read];
    if head != ELF_MAGIC && !is_sce_wrapped(head) {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if head == ELF_MAGIC {
        return Ok(Some(bytes));
    }
    decrypt_ppu_self(&bytes, &path.display().to_string(), vfs_root)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn print_human(reports: &[TitleReport], total: &SpuCensus, images: usize) {
    for report in reports {
        match &report.skipped {
            Some(reason) => println!("title {}: skipped -- {reason}", report.name),
            None => println!(
                "title {}: {} file(s) read, {} new SPU image(s), {} code word(s)",
                report.name, report.files, report.images, report.words
            ),
        }
        for (path, reason) in &report.unreadable {
            println!("  unread: {} -- {reason}", path.display());
        }
    }
    let words = total.words();
    println!("spu-census: {images} SPU image(s), {words} code word(s)");
    for class in CLASSES {
        let n = total.words_in(class);
        println!(
            "  {:<16} {n:>10}  {:>6.2}%",
            class.label(),
            percent(n, words)
        );
    }
    println!("per mnemonic:");
    for row in total.rows() {
        println!(
            "  {:<10} {:<16} {:>10}",
            row.row.mnemonic,
            row.class.label(),
            row.words
        );
    }
}

fn print_json(
    reports: &[TitleReport],
    total: &SpuCensus,
    images: usize,
) -> Result<(), CommandError> {
    let titles: Vec<serde_json::Value> = reports
        .iter()
        .map(|report| {
            serde_json::json!({
                "title": report.name,
                "skipped": report.skipped,
                "files": report.files,
                "images": report.images,
                "words": report.words,
                "unread": report.unreadable.iter().map(|(path, reason)| serde_json::json!({
                    "path": path.display().to_string(),
                    "reason": reason,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let classes: serde_json::Map<String, serde_json::Value> = CLASSES
        .iter()
        .map(|&class| (class.label().to_string(), total.words_in(class).into()))
        .collect();
    let mnemonics: Vec<serde_json::Value> = total
        .rows()
        .map(|row| {
            serde_json::json!({
                "mnemonic": row.row.mnemonic,
                "class": row.class.label(),
                "words": row.words,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "titles": titles,
        "images": images,
        "words": total.words(),
        "classes": classes,
        "mnemonics": mnemonics,
    });
    let text = serde_json::to_string_pretty(&doc)
        .map_err(|error| CommandError::failed(format!("spu-census: json: {error}")))?;
    println!("{text}");
    Ok(())
}
