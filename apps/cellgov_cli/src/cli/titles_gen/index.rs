//! `docs/titles.md`: one row per title, measured at the cell its
//! manifest marks the reference, plus a coverage count over every
//! declared cell.
//!
//! The Config column names that cell, so a reader can tell which
//! firmware and game version produced a step count.

use std::collections::BTreeSet;

use cellgov_compare::{format_with_commas, BootSummary};

use super::detail::detail_page_link;
use super::load::TitleDocs;
use crate::game::manifest::TitleManifest;

const TITLES_TEMPLATE: &str = include_str!("../templates/titles.md.template");

/// Rendered in every data cell of a title that has nothing recorded at
/// its reference cell.
const NO_DATA: &str = "--";

/// Render the whole `docs/titles.md` body.
pub(crate) fn render(docs: &[TitleDocs<'_>]) -> String {
    let rows: Vec<String> = docs.iter().map(render_row).collect();
    super::super::fixture_gen::apply_subs(
        TITLES_TEMPLATE,
        &[
            ("matrix_rows", &rows.join("\n")),
            ("coverage", &render_coverage(docs)),
        ],
    )
}

/// The index's one-line coverage count: how much of the declared space
/// carries a result.
fn render_coverage(docs: &[TitleDocs<'_>]) -> String {
    let firmwares: BTreeSet<&str> = docs
        .iter()
        .flat_map(|d| d.cells.iter().map(|(key, _)| key.fw.as_str()))
        .collect();
    let declared: usize = docs.iter().map(|d| d.cells.len()).sum();
    let recorded = docs
        .iter()
        .flat_map(|d| d.cells.iter())
        .filter(|(_, result)| result.is_recorded())
        .count();
    format!(
        "{} title(s), {} firmware(s), {declared} declared cell(s), {recorded} recorded.",
        docs.len(),
        firmwares.len(),
    )
}

/// One markdown table row, at the title's reference cell.
fn render_row(docs: &TitleDocs<'_>) -> String {
    let title = docs.title;
    let (checkpoint_cell, steps_cell, insns_cell) = match &docs.reference.boot {
        Some(b) => (
            format_checkpoint(b),
            format_with_commas(b.steps),
            format_with_commas(b.insns()),
        ),
        None => (
            NO_DATA.to_string(),
            NO_DATA.to_string(),
            NO_DATA.to_string(),
        ),
    };
    let (convergence_cell, byte_parity_cell) = match &docs.reference.cross {
        Some(c) => c.display_matrix_columns(),
        None => (NO_DATA.to_string(), NO_DATA.to_string()),
    };
    let config_cell = title
        .reference_cell()
        .map_or_else(|| NO_DATA.to_string(), |c| c.key.label());

    assert_table_safe("title manifest field `content_id`", &title.content_id);
    assert_table_safe("title manifest field `display_name`", &title.display_name);
    assert_table_safe("title manifest field `developer`", &title.developer);
    assert_table_safe("title manifest field `engine`", &title.engine);
    assert_table_safe("the reference cell's label", &config_cell);
    // These three quote a committed summary; no loader checks a
    // summary against the table's rules.
    assert_table_safe("the reference cell's checkpoint", &checkpoint_cell);
    assert_table_safe("the reference cell's convergence", &convergence_cell);
    assert_table_safe("the reference cell's byte parity", &byte_parity_cell);

    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        detail_page_link(&title.content_id),
        title.display_name,
        title.year,
        title.developer,
        title.engine,
        title.distribution.format_label(),
        config_cell,
        checkpoint_cell,
        steps_cell,
        insns_cell,
        convergence_cell,
        byte_parity_cell,
    )
}

/// `<checkpoint kind> -> <observed outcome>`; both columns render so
/// a regressed run (`FirstRsxWrite -> Fault`) is visibly distinct
/// from a clean one (`FirstRsxWrite -> RsxWriteCheckpoint`).
fn format_checkpoint(b: &BootSummary) -> String {
    format!("{} -> {}", b.checkpoint.as_markdown_label(), b.outcome)
}

/// The order every generated document lists titles in: `content_id`
/// ascending.
pub(crate) fn sort_by_content_id<'a>(
    titles: impl IntoIterator<Item = &'a TitleManifest>,
) -> Vec<&'a TitleManifest> {
    let mut titles: Vec<&TitleManifest> = titles.into_iter().collect();
    titles.sort_by(|a, b| a.content_id.cmp(&b.content_id));
    debug_assert!(
        titles.windows(2).all(|w| w[0].content_id < w[1].content_id),
        "titles-gen: duplicate content_id in registry"
    );
    titles
}

/// Refuse a `|` or a line break in a value bound for a table cell.
///
/// The documented way to regenerate these documents is a release
/// build. A debug-only check is absent there, so a broken row reaches
/// the committed document as the generator's answer. The manifest
/// loader refuses `[[bench.matrix]] pending` for the same reason.
///
/// # Panics
///
/// Panics when `value` holds a `|`, an LF, or a CR. A markdown line
/// ending is an LF, a CRLF, or a bare CR, so a lone CR ends the row
/// as an LF does.
pub(crate) fn assert_table_safe(field: &str, value: &str) {
    assert!(
        !value.contains('|') && !value.contains('\n') && !value.contains('\r'),
        "`{field}` contains markdown-table-breaking char(s): {value:?}"
    );
}

#[cfg(test)]
#[path = "tests/index_tests.rs"]
mod tests;
