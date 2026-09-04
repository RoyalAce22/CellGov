//! `cellgov status` -- what this machine holds, and which cells have an
//! anchor.

use std::io::IsTerminal;
use std::path::Path;

use crate::cli::parse::OutputFormat;

use super::model::{StatusDoc, TitleDoc};
use super::{emit, human_bytes, tree_bytes, view};

/// `cellgov status`
pub(crate) fn status(root: &Path, format: OutputFormat, quiet: bool) {
    let view = view(root);
    let mut titles = view.title_docs();
    // A declared title with nothing installed still has cells, and its
    // absence is the answer to "why does no anchor exist for it".
    for manifest in view.registry.iter() {
        if !titles.iter().any(|t| t.title_id == manifest.content_id) {
            titles.push(view.declared_only_doc(manifest));
        }
    }
    titles.sort_by(|a, b| a.title_id.cmp(&b.title_id));

    let size = tree_bytes(root);
    let doc = StatusDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        store_bytes: size.bytes,
        unreadable_paths: size.unreadable,
        firmware: view.firmware_docs(),
        titles,
    };
    emit(format, &doc, || {
        print!("{}", render(&doc));
        if !quiet {
            hint(&doc);
        }
    });
}

fn render(doc: &StatusDoc) -> String {
    let mut out = format!("store  {}  ({})\n\n", doc.store, size_label(doc));

    if doc.firmware.is_empty() {
        out.push_str("firmware   none installed\n");
    } else {
        for (i, entry) in doc.firmware.iter().enumerate() {
            out.push_str(&format!(
                "{:<10} {:<7} {} module(s)  image {}\n",
                if i == 0 { "firmware" } else { "" },
                entry.version,
                entry
                    .modules
                    .map_or_else(|| "--".to_string(), |n| n.to_string()),
                entry.image_version.as_deref().unwrap_or("--"),
            ));
        }
    }
    out.push('\n');

    if doc.titles.is_empty() {
        out.push_str("titles     none installed and none declared\n");
    } else {
        for (i, title) in doc.titles.iter().enumerate() {
            out.push_str(&format!(
                "{:<10} {:<9}  {:<12} {}\n",
                if i == 0 { "titles" } else { "" },
                title.title_id,
                title.short_name.as_deref().unwrap_or("<no manifest>"),
                installed_summary(title),
            ));
        }
    }

    let cells: Vec<(&TitleDoc, &super::model::AnchorDoc)> = doc
        .titles
        .iter()
        .flat_map(|t| t.anchors.iter().map(move |c| (t, c)))
        .collect();
    if !cells.is_empty() {
        out.push('\n');
        for (i, (title, cell)) in cells.iter().enumerate() {
            out.push_str(&format!(
                "{:<10} {:<12} fw {} x {:<6} {}\n",
                if i == 0 { "anchors" } else { "" },
                title.short_name.as_deref().unwrap_or(&title.title_id),
                cell.fw,
                cell.game_ver.as_deref().unwrap_or("--"),
                if cell.recorded { "recorded" } else { "none" },
            ));
        }
    }
    out
}

/// The store size, marked as a floor when the walk could not read a
/// path.
fn size_label(doc: &StatusDoc) -> String {
    match doc.unreadable_paths {
        0 => human_bytes(doc.store_bytes),
        n => format!(
            "at least {}; {n} path(s) could not be read",
            human_bytes(doc.store_bytes)
        ),
    }
}

/// What of a title is on this machine, as the titles block prints it.
fn installed_summary(title: &TitleDoc) -> String {
    match (&title.base, title.updates.len()) {
        // A title shipped inside the firmware has no store entry of its
        // own, so its installed state is its firmware's.
        (None, 0) if title.ships_in_firmware => {
            if title.anchors.iter().any(|c| c.installed) {
                "ships inside the firmware".to_string()
            } else {
                "ships inside the firmware, none of whose versions is installed".to_string()
            }
        }
        (None, 0) => "declared, nothing installed".to_string(),
        (None, n) => format!("{n} update(s), no base -- orphan"),
        (Some(base), 0) => format!("{} base {}", base.distribution, base.app_ver),
        (Some(base), _) => format!(
            "{} base {} + updates: {}",
            base.distribution,
            base.app_ver,
            title
                .updates
                .iter()
                .map(|u| u.version.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }
}

/// Print at most one next step, and only when it is unambiguous.
///
/// It goes to stderr so a piped stdout carries the report alone.
fn hint(doc: &StatusDoc) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    let Some(line) = next_step(doc) else { return };
    eprintln!();
    eprintln!("next:  {line}");
}

/// The one command worth suggesting, or `None` when several are.
fn next_step(doc: &StatusDoc) -> Option<String> {
    if doc.firmware.is_empty() {
        return Some("cellgov firmware install <PS3UPDAT.PUP>".to_string());
    }
    // A title shipped inside the firmware boots off the firmware alone
    // and has no base to install.
    let bootable = doc
        .titles
        .iter()
        .any(|t| t.base.is_some() || t.anchors.iter().any(|c| c.installed));
    if !bootable {
        return Some("cellgov title install <PKG|ISO>".to_string());
    }
    // The one bootable cell with no anchor. Two of them is a choice
    // this line cannot make for the operator.
    let mut unrecorded = doc.titles.iter().flat_map(|title| {
        title
            .anchors
            .iter()
            .filter(|c| c.installed && !c.recorded)
            .map(move |c| (title, c))
    });
    let (title, cell) = unrecorded.next()?;
    if unrecorded.next().is_some() {
        return None;
    }
    let selector = title
        .short_name
        .clone()
        .unwrap_or_else(|| title.title_id.clone());
    Some(match &cell.game_ver {
        Some(game_ver) => format!(
            "cellgov boot bench --title {selector} --fw {} --game-ver {game_ver}   (no anchor yet)",
            cell.fw
        ),
        None => format!(
            "cellgov boot bench --title {selector} --fw {}   (no anchor yet)",
            cell.fw
        ),
    })
}

#[cfg(test)]
#[path = "tests/status_tests.rs"]
mod tests;
