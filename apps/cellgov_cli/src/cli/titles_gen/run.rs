//! Renders the whole output set and writes it to the output directory.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use super::detail::{detail_page_path, DETAIL_DIR};
use super::load::{load_title, SummaryLoadError};
use super::{detail, firmware, index};
use crate::cli::exit::die;
use crate::cli::parse::TitlesGenArgs;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use cellgov_boot::manifest::{TitleManifest, TitleRegistry};

/// Directory the generated documents are written under.
const DEFAULT_OUTPUT_DIR: &str = "docs";

/// The title index, which sits at the output directory's root.
const INDEX_FILE: &str = "titles.md";

/// The firmware page, beside the index.
const FIRMWARE_FILE: &str = "firmware.md";

/// One generated file, at a path relative to the output directory.
pub(crate) struct GeneratedDoc {
    pub(crate) path: PathBuf,
    pub(crate) body: String,
}

pub(crate) fn run(args: &TitlesGenArgs) {
    let registry_dir = args
        .registry
        .clone()
        .unwrap_or_else(|| DEFAULT_TITLE_REGISTRY_DIR.to_string());
    let fixtures_dir = args
        .fixtures_dir
        .clone()
        .unwrap_or_else(|| crate::paths::DEFAULT_FIXTURES_DIR.to_string());
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_string());
    // An empty path reads as the working directory, and this command
    // sweeps `<output-dir>/titles/` for pages to delete.
    if output_dir.is_empty() {
        die(
            "titles-gen: --output-dir is empty; name the directory the documents are written under",
        );
    }

    let registry = TitleRegistry::scan_dir(Path::new(&registry_dir))
        .unwrap_or_else(|e| die(&format!("titles-gen: scan {registry_dir}: {e}")));

    let titles = registry.iter().count();
    let docs = render_docs(registry.iter(), Path::new(&fixtures_dir))
        .unwrap_or_else(|e| die(&format!("titles-gen: {e}")));

    let out = Path::new(&output_dir);
    for doc in &docs {
        let path = out.join(&doc.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| die(&format!("titles-gen: create {}: {e}", parent.display())));
        }
        std::fs::write(&path, &doc.body)
            .unwrap_or_else(|e| die(&format!("titles-gen: write {}: {e}", path.display())));
    }
    for orphan in orphaned_pages(out, &docs).unwrap_or_else(|e| {
        die(&format!(
            "titles-gen: list {}: {e}",
            out.join(DETAIL_DIR).display()
        ))
    }) {
        std::fs::remove_file(&orphan)
            .unwrap_or_else(|e| die(&format!("titles-gen: remove {}: {e}", orphan.display())));
        println!(
            "titles-gen: removed {} (no title declares it)",
            orphan.display()
        );
    }
    println!(
        "titles-gen: wrote {} file(s) under {output_dir} ({titles} title(s))",
        docs.len(),
    );
}

/// Render every file the generator owns: the title index, the firmware
/// page, then one page per title in `content_id`-ascending order.
///
/// [`run`] and the committed-doc drift gate both call this, so the
/// gate checks the bytes and the file set the generator writes.
///
/// # Errors
///
/// [`SummaryLoadError`] if a title's committed artifacts cannot be
/// read, disagree with the cell they sit in, or name a cell the
/// manifest does not declare.
pub(crate) fn render_docs<'a>(
    titles: impl IntoIterator<Item = &'a TitleManifest>,
    fixtures: &Path,
) -> Result<Vec<GeneratedDoc>, SummaryLoadError> {
    let titles = index::sort_by_content_id(titles);
    let loaded = titles
        .iter()
        .map(|t| load_title(t, fixtures))
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = vec![
        GeneratedDoc {
            path: PathBuf::from(INDEX_FILE),
            body: index::render(&loaded),
        },
        GeneratedDoc {
            path: PathBuf::from(FIRMWARE_FILE),
            body: firmware::render(&loaded),
        },
    ];
    out.extend(loaded.iter().map(|d| GeneratedDoc {
        path: detail_page_path(&d.title.content_id),
        body: detail::render(d),
    }));
    Ok(out)
}

/// Pages under the detail directory that no title in `docs` claims.
///
/// The sweep covers that directory alone, and only its markdown
/// files. The generator owns the directory outright, so a file in it
/// that nothing generates is residue from a title the registry
/// dropped.
///
/// The sweep compares the file each path resolves to. On a
/// case-insensitive filesystem the directory can list a page written
/// as `NPAA00001.md` back as `npaa00001.md`, and a comparison of the
/// two spellings deletes a page this run wrote.
///
/// # Errors
///
/// The directory listing failure. A directory that does not exist
/// holds no orphans.
fn orphaned_pages(out: &Path, docs: &[GeneratedDoc]) -> Result<Vec<PathBuf>, io::Error> {
    let dir = out.join(DETAIL_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let owned: BTreeSet<PathBuf> = docs.iter().map(|d| resolved(&out.join(&d.path))).collect();
    let mut orphans = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "md") && !owned.contains(&resolved(&path)) {
            orphans.push(path);
        }
    }
    orphans.sort();
    Ok(orphans)
}

/// The file `path` resolves to, or `path` itself when it names no
/// file.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "tests/run_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/resolution_tests.rs"]
mod resolution_tests;
