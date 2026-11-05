//! `dev ppu-census`: the PPU instruction words of installed titles,
//! counted by architectural unit and by what CellGov does with each.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::TitleManifest;
use cellgov_fuzz::ppu_census::{PpuCensus, PpuCensusRecord, PpuCensusTitle};
use cellgov_install::store::StoreInventory;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::{OutputFormat, PpuCensusArgs};
use crate::title_images::{census_inputs, read_image, title_files};

/// What the census read of one title.
struct TitleReport {
    title: PpuCensusTitle,
    /// Each file or directory the census could not read, and why.
    unreadable: Vec<(PathBuf, String)>,
}

pub(crate) fn run(
    args: &PpuCensusArgs,
    vfs_flag: Option<&Path>,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let inputs = census_inputs(&args.scope, vfs_flag, "ppu-census")?;
    let mut seen = BTreeSet::new();
    let mut total = PpuCensus::new();
    let mut reports = Vec::new();
    for title in &inputs.titles {
        reports.push(census_title(
            title,
            &inputs.inventory,
            &inputs.vfs_root,
            &mut seen,
            &mut total,
        ));
    }
    let titles = reports.iter().map(|report| report.title.clone()).collect();
    let record = PpuCensusRecord::new(titles, &total);
    match format {
        OutputFormat::Human => print_human(&reports, &total),
        OutputFormat::Json => print_json(&record)?,
    }
    Ok(CommandExitCode::SUCCESS)
}

/// Count the PPU images of one title that no earlier title held.
fn census_title(
    title: &TitleManifest,
    inventory: &StoreInventory,
    vfs_root: &Path,
    seen: &mut BTreeSet<u64>,
    total: &mut PpuCensus,
) -> TitleReport {
    let mut report = TitleReport {
        title: PpuCensusTitle {
            content_id: title.content_id.clone(),
            skipped: None,
            images: 0,
            words: 0,
            unread: 0,
        },
        unreadable: Vec::new(),
    };
    let found = match title_files(title, inventory, vfs_root) {
        Ok(found) => found,
        Err(reason) => {
            report.title.skipped = Some(reason.to_string());
            return report;
        }
    };
    report.unreadable = found.unreadable;
    for path in found.files {
        let counted = match read_image(&path, vfs_root) {
            Ok(Some(bytes)) => total
                .add_new_elf(&bytes, seen)
                .map_err(|error| error.to_string()),
            Ok(None) => Ok(None),
            Err(error) => Err(error.to_string()),
        };
        match counted {
            Ok(Some(words)) => {
                report.title.images += 1;
                report.title.words += words;
            }
            Ok(None) => {}
            Err(reason) => report.unreadable.push((path, reason)),
        }
    }
    report.title.unread = report.unreadable.len();
    report
}

fn print_human(reports: &[TitleReport], total: &PpuCensus) {
    for report in reports {
        let title = &report.title;
        match &title.skipped {
            Some(reason) => println!("title {}: skipped -- {reason}", title.content_id),
            None => println!(
                "title {}: {} new PPU image(s), {} code word(s)",
                title.content_id, title.images, title.words
            ),
        }
        for (path, reason) in &report.unreadable {
            println!("  unread: {} -- {reason}", path.display());
        }
    }
    println!(
        "ppu-census: {} code word(s) in {} unit(s)",
        total.words(),
        total.units().len()
    );
    for (unit, counts) in total.units() {
        let refused: Vec<String> = counts
            .refused
            .iter()
            .map(|(class, words)| format!("{class}={words}"))
            .collect();
        println!(
            "  {unit:<28} seen={:<9} implemented={:<9} refused=[{}] not_recognized={}",
            counts.seen(),
            counts.implemented,
            refused.join(" "),
            counts.not_recognized
        );
    }
}

fn print_json(record: &PpuCensusRecord) -> Result<(), CommandError> {
    let text = serde_json::to_string_pretty(record)
        .map_err(|error| CommandError::failed(format!("ppu-census: json: {error}")))?;
    println!("{text}");
    Ok(())
}
