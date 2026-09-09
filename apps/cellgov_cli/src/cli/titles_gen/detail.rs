//! `docs/titles/<content-id>.md`: one title's declared matrix as a
//! grid, firmware down the side and game version across.
//!
//! A blank cell means out of scope. A `.` cell means declared and
//! unmeasured.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::cell::CellResult;
use super::index::assert_table_safe;
use super::load::TitleDocs;
use crate::game::manifest::{CellKey, BASE_GAME_VER};

const DETAIL_TEMPLATE: &str = include_str!("../templates/title_detail.md.template");

/// Directory the per-title pages sit in, under the output directory.
pub(crate) const DETAIL_DIR: &str = "titles";

/// Marks the reference cell -- the one the index renders.
const REFERENCE_MARK: &str = "*";

/// The index a game title's page points back to, from inside
/// [`DETAIL_DIR`].
const INDEX_LINK: &str = "the [matrix](../titles.md)";

/// The page a firmware-shipped title's page points back to, from
/// inside [`DETAIL_DIR`].
const FIRMWARE_LINK: &str = "the [firmware page](../firmware.md)";

/// The index's link to one title's page.
pub(crate) fn detail_page_link(content_id: &str) -> String {
    assert_page_name_safe(content_id);
    format!("[{content_id}]({DETAIL_DIR}/{content_id}.md)")
}

/// Where one title's page is written, relative to the output
/// directory.
pub(crate) fn detail_page_path(content_id: &str) -> PathBuf {
    assert_page_name_safe(content_id);
    PathBuf::from(DETAIL_DIR).join(format!("{content_id}.md"))
}

/// Refuse a content id that is not one file name and one bare link
/// target.
///
/// The manifest loader takes the field as free text. It lands here as
/// a path component under [`DETAIL_DIR`] and as the target of the
/// index's link. A separator or a `..` writes the page outside the
/// directory the generator owns and sweeps. A bracket or a space ends
/// the link early.
///
/// # Panics
///
/// Panics when `content_id` is empty, is `.` or `..`, or holds a
/// character outside ASCII alphanumerics, `-`, `_`, and `.`.
fn assert_page_name_safe(content_id: &str) {
    assert!(
        !content_id.is_empty()
            && content_id != "."
            && content_id != ".."
            && content_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "title manifest field `content_id` is neither one file name nor a bare markdown link \
         target: {content_id:?}"
    );
}

/// Render one title's detail page.
pub(crate) fn render(docs: &TitleDocs<'_>) -> String {
    let title = docs.title;
    assert_table_safe("title manifest field `content_id`", &title.content_id);
    assert_table_safe("title manifest field `display_name`", &title.display_name);
    let back = if docs.ships_in_firmware() {
        FIRMWARE_LINK
    } else {
        INDEX_LINK
    };
    super::super::fixture_gen::apply_subs(
        DETAIL_TEMPLATE,
        &[
            ("content_id", &title.content_id),
            ("display_name", &title.display_name),
            ("index_link", back),
            ("grid", &render_grid(docs)),
        ],
    )
}

/// The grid, or a line saying the title declares no cells.
fn render_grid(docs: &TitleDocs<'_>) -> String {
    if docs.cells.is_empty() {
        return "This title declares no cells.".to_string();
    }
    let reference = docs.title.reference_key();
    let by_cell: BTreeMap<&CellKey, &CellResult> =
        docs.cells.iter().map(|c| (&c.key, &c.result)).collect();
    let firmwares: BTreeSet<&str> = docs.cells.iter().map(|c| c.key.fw.as_str()).collect();

    // A title shipped inside the firmware has no game-version axis, so
    // its grid is one column of results.
    let games = game_versions(docs);
    let mut out = match games.as_slice() {
        [] => vec!["| fw | result |".to_string(), "| --- | --- |".to_string()],
        gs => vec![
            format!("| fw \\ game | {} |", gs.join(" | ")),
            format!("| --- |{}", " --- |".repeat(gs.len())),
        ],
    };
    for fw in firmwares {
        let mut row = vec![fw.to_string()];
        match games.as_slice() {
            [] => row.push(render_token(&by_cell, reference.as_ref(), fw, None)),
            gs => row.extend(
                gs.iter()
                    .map(|g| render_token(&by_cell, reference.as_ref(), fw, Some(g))),
            ),
        }
        out.push(format!("| {} |", row.join(" | ")));
    }
    out.join("\n")
}

/// Every game version the title declares a cell for, `base` first.
///
/// `base` names the title's own install, so it leads whatever the
/// update keys sort as. The list is empty for a firmware-shipped
/// title.
fn game_versions<'a>(docs: &'a TitleDocs<'_>) -> Vec<&'a str> {
    let declared: BTreeSet<&str> = docs
        .cells
        .iter()
        .filter_map(|c| c.key.game_ver.as_deref())
        .collect();
    let mut out: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|v| *v != BASE_GAME_VER)
        .collect();
    if declared.contains(BASE_GAME_VER) {
        out.insert(0, BASE_GAME_VER);
    }
    out
}

/// One grid cell: the declared cell's token, or blank when the title
/// declares no cell at this intersection.
fn render_token(
    by_cell: &BTreeMap<&CellKey, &CellResult>,
    reference: Option<&CellKey>,
    fw: &str,
    game_ver: Option<&str>,
) -> String {
    let key = CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    };
    match by_cell.get(&key) {
        None => String::new(),
        Some(result) => {
            let mark = if reference == Some(&key) {
                REFERENCE_MARK
            } else {
                ""
            };
            let token = format!("{}{mark}", result.token());
            // A token quotes a committed summary's reason or outcome;
            // no loader checks a summary against the table's rules.
            assert_table_safe("the grid cell for this title", &token);
            token
        }
    }
}

#[cfg(test)]
#[path = "tests/detail_tests.rs"]
mod tests;
