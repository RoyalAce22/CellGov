//! `firmware list | show` and `title list | show`.
//!
//! `list` reports what the store holds; `show` reports one entry of it
//! in full. Both render the same documents.

use std::path::Path;

use crate::cli::exit::die;
use crate::cli::parse::OutputFormat;

use super::model::{
    BaseDoc, CoreOsDoc, FirmwareDoc, FirmwareListDoc, TitleDoc, TitleListDoc, KERNEL_NOT_RECORDED,
};
use super::{emit, view};

/// The label a title renders under when no registry manifest names it.
const NO_MANIFEST: &str = "<no manifest>";

/// What a record line reads when the key names no record path.
const NO_RECORD: &str = "-- (the key is not a store directory name)";

/// `cellgov firmware list`
pub(crate) fn firmware_list(root: &Path, format: OutputFormat) {
    let view = view(root);
    let doc = FirmwareListDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        firmware: view.firmware_docs(),
    };
    emit(format, &doc, || print!("{}", render_firmware_list(&doc)));
}

/// `cellgov firmware show <VERSION>`
pub(crate) fn firmware_show(root: &Path, version: &str, format: OutputFormat) {
    let view = view(root);
    let entry = view.inventory.firmware(version).unwrap_or_else(|| {
        die(&format!(
            "no firmware {version:?} is installed; installed: {}",
            super::key_list(&view.inventory.firmware_versions())
        ))
    });
    let doc = FirmwareListDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        firmware: vec![view.firmware_doc(entry)],
    };
    emit(format, &doc, || {
        for entry in &doc.firmware {
            print!("{}", render_firmware_detail(entry));
        }
    });
}

/// `cellgov title list`
pub(crate) fn title_list(root: &Path, format: OutputFormat) {
    let view = view(root);
    let doc = TitleListDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        titles: view.title_docs(),
    };
    emit(format, &doc, || print!("{}", render_title_list(&doc)));
}

/// `cellgov title show <TITLE_ID>`
pub(crate) fn title_show(root: &Path, title_id: &str, format: OutputFormat) {
    let view = view(root);
    let entry = view.inventory.title(title_id).unwrap_or_else(|| {
        die(&format!(
            "no title {title_id:?} is installed; installed: {}",
            super::key_list(
                &view
                    .inventory
                    .titles()
                    .map(|t| t.title_id.clone())
                    .collect::<Vec<_>>()
            )
        ))
    });
    let doc = TitleListDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        titles: vec![view.title_doc(entry)],
    };
    emit(format, &doc, || {
        for title in &doc.titles {
            print!("{}", render_title_detail(title));
        }
    });
}

fn render_firmware_list(doc: &FirmwareListDoc) -> String {
    if doc.firmware.is_empty() {
        return format!("no firmware installed under {}\n", doc.store);
    }
    let mut out = String::from("  VERSION  MODULES  KERNEL  IMAGE\n");
    for entry in &doc.firmware {
        out.push_str(&format!(
            "  {:<7}  {:>7}  {:<6}  {}\n",
            entry.version,
            entry
                .modules
                .map_or_else(|| "--".to_string(), |n| n.to_string()),
            if has_kernel(entry) { "yes" } else { "--" },
            entry.image_version.as_deref().unwrap_or("--"),
        ));
    }
    out
}

fn has_kernel(entry: &FirmwareDoc) -> bool {
    entry
        .core_os
        .as_ref()
        .is_some_and(|block| block.kernel.is_some())
}

/// The `kernel` line of a firmware detail: where the kernel is, or why
/// there is none.
fn kernel_label(core_os: Option<&CoreOsDoc>) -> String {
    match core_os {
        Some(CoreOsDoc {
            kernel: Some(kernel),
            ..
        }) => format!(
            "{} (as stored, sha256 {})",
            kernel.path, kernel.stored_sha256
        ),
        Some(CoreOsDoc {
            omission: Some(why),
            ..
        }) => format!("not unpacked ({why})"),
        Some(_) => "not unpacked".to_string(),
        None => KERNEL_NOT_RECORDED.to_string(),
    }
}

fn render_firmware_detail(entry: &FirmwareDoc) -> String {
    let mut out = format!("firmware {}\n", entry.version);
    out.push_str(&format!("  entry      {}\n", entry.entry_dir));
    out.push_str(&format!(
        "  record     {}\n",
        entry.record.as_deref().unwrap_or(NO_RECORD)
    ));
    out.push_str(&format!("  pup sha256 {}\n", entry.pup_sha256));
    if let Some(image) = &entry.image_version {
        out.push_str(&format!("  image      {image}\n"));
    }
    let modules = match (entry.modules, &entry.manifest_error) {
        (Some(n), _) => format!("{n} covered by firmware.toml"),
        (None, Some(why)) => format!("-- ({why})"),
        (None, None) => "-- (the mount holds no readable firmware.toml)".to_string(),
    };
    out.push_str(&format!("  modules    {modules}\n"));
    out.push_str(&format!(
        "  kernel     {}\n",
        kernel_label(entry.core_os.as_ref())
    ));
    if let Some(block) = entry.core_os.as_ref().filter(|b| !b.files.is_empty()) {
        out.push_str(&format!(
            "  core os    {} file(s) in the package table\n",
            block.files.len()
        ));
    }
    out
}

fn render_title_list(doc: &TitleListDoc) -> String {
    if doc.titles.is_empty() {
        return format!("no title installed under {}\n", doc.store);
    }
    // The base column is wide enough for a version under its longer
    // key and for the no-version label, so neither pushes its row out
    // of line.
    let mut out = format!(
        "  {:<9}  {:<20}  {:<17}  UPDATES\n",
        "TITLE ID", "NAME", "BASE"
    );
    for title in &doc.titles {
        let updates = if title.updates.is_empty() {
            "--".to_string()
        } else {
            title
                .updates
                .iter()
                .map(|u| u.version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "  {:<9}  {:<20}  {:<17}  {updates}\n",
            title.title_id,
            title.short_name.as_deref().unwrap_or(NO_MANIFEST),
            title
                .base
                .as_ref()
                .map_or_else(|| "--".to_string(), BaseDoc::version_label),
        ));
    }
    for title in &doc.titles {
        for note in orphan_notes(title) {
            out.push_str(&format!("  {note}\n"));
        }
    }
    out
}

/// The lines flagging a title the store holds only part of, or that no
/// registry manifest names.
fn orphan_notes(title: &TitleDoc) -> Vec<String> {
    let mut out = Vec::new();
    if title.short_name.is_none() {
        out.push(format!(
            "{}: orphan -- no {}/*.toml declares this title, so no cell names it",
            title.title_id,
            crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR
        ));
    }
    if title.base.is_none() && !title.updates.is_empty() {
        out.push(format!(
            "{}: orphan -- {} update(s) installed with no base to patch",
            title.title_id,
            title.updates.len()
        ));
    }
    out
}

fn render_title_detail(title: &TitleDoc) -> String {
    let mut out = format!(
        "title {}  {}\n",
        title.title_id,
        title.display_name.as_deref().unwrap_or(NO_MANIFEST)
    );
    match &title.base {
        Some(base) => {
            out.push_str(&format!(
                "  base       {} ({}, {} tree)\n",
                base.version_label(),
                base.distribution,
                base.tree
            ));
            if let Some(why) = &base.param_sfo_error {
                out.push_str(&format!("  param.sfo  {why}\n"));
            }
            out.push_str(&format!("  dir        {}\n", base.dir));
            out.push_str(&format!(
                "  record     {}\n",
                base.record.as_deref().unwrap_or(NO_RECORD)
            ));
            out.push_str(&format!("  source     {}\n", base.source_sha256));
            if let Some(v) = &base.system_ver {
                out.push_str(&format!("  needs fw   {v}\n"));
            }
            if let Some(v) = &base.shipped_firmware {
                out.push_str(&format!("  ships fw   {v}\n"));
            }
        }
        None => out.push_str("  base       -- (not installed)\n"),
    }
    for update in &title.updates {
        out.push_str(&format!("  update {}\n", update.version_label()));
        if let Some(why) = &update.param_sfo_error {
            out.push_str(&format!("    param.sfo {why}\n"));
        }
        out.push_str(&format!("    dir      {}\n", update.dir));
        out.push_str(&format!(
            "    record   {}\n",
            update.record.as_deref().unwrap_or(NO_RECORD)
        ));
        out.push_str(&format!("    source   {}\n", update.source_sha256));
        if let Some(v) = &update.system_ver {
            out.push_str(&format!("    needs fw {v}\n"));
        }
        if let Some(min) = &update.min_system_ver {
            out.push_str(&format!("    min fw   {min}\n"));
        }
    }
    if !title.anchors.is_empty() {
        out.push_str("  cells\n");
        for cell in &title.anchors {
            out.push_str(&format!(
                "    fw {} x {}  {}{}{}\n",
                cell.fw,
                cell.game_ver.as_deref().unwrap_or("--"),
                if cell.recorded {
                    "recorded"
                } else {
                    "no anchor"
                },
                if cell.reference { ", reference" } else { "" },
                if cell.installed {
                    ""
                } else {
                    ", not installed here"
                },
            ));
        }
    }
    for note in orphan_notes(title) {
        out.push_str(&format!("  {note}\n"));
    }
    out
}

#[cfg(test)]
#[path = "tests/list_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/version_label_tests.rs"]
mod version_label_tests;

#[cfg(test)]
#[path = "tests/shipped_firmware_tests.rs"]
mod shipped_firmware_tests;
