//! `docs/titles.md`: one row per game title at its reference cell,
//! plus a coverage count over every cell those titles declare. The
//! reference cell is the title's floor times its base install.
//!
//! The Config column names that cell, so a reader can tell which
//! firmware and game version produced a step count. A title shipped
//! inside the firmware has no such cell and renders on the firmware
//! page instead ([`super::firmware`]).

use std::collections::BTreeSet;

use cellgov_compare::{format_with_commas, BootSummary};

use super::cell::CellArtifacts;
use super::detail::detail_page_link;
use super::load::TitleDocs;
use crate::game::manifest::TitleManifest;

const TITLES_TEMPLATE: &str = include_str!("../templates/titles.md.template");

/// Rendered in a table cell that has nothing to quote.
pub(super) const NO_DATA: &str = "--";

/// Render the whole `docs/titles.md` body from the game titles in
/// `docs`.
pub(crate) fn render(docs: &[TitleDocs<'_>]) -> String {
    let games: Vec<&TitleDocs<'_>> = docs.iter().filter(|d| !d.ships_in_firmware()).collect();
    let rows: Vec<String> = games.iter().map(|d| render_row(d)).collect();
    super::super::fixture_gen::apply_subs(
        TITLES_TEMPLATE,
        &[
            ("matrix_rows", &rows.join("\n")),
            ("coverage", &render_coverage(&games)),
        ],
    )
}

/// The index's one-line coverage count: how much of the declared space
/// carries a result, over the game titles alone.
fn render_coverage(games: &[&TitleDocs<'_>]) -> String {
    let (firmwares, declared, recorded) = coverage_counts(games.iter().copied());
    format!(
        "{} game title(s), {firmwares} firmware(s), {declared} declared cell(s), {recorded} \
         recorded.",
        games.len(),
    )
}

/// `(distinct firmwares, declared cells, recorded cells)` over `docs`.
pub(super) fn coverage_counts<'a>(
    docs: impl IntoIterator<Item = &'a TitleDocs<'a>>,
) -> (usize, usize, usize) {
    let mut firmwares: BTreeSet<&str> = BTreeSet::new();
    let mut declared = 0;
    let mut recorded = 0;
    for doc in docs {
        for cell in &doc.cells {
            firmwares.insert(cell.key.fw.as_str());
            declared += 1;
            if cell.result.is_recorded() {
                recorded += 1;
            }
        }
    }
    (firmwares.len(), declared, recorded)
}

/// One markdown table row, at the title's reference cell.
fn render_row(docs: &TitleDocs<'_>) -> String {
    let title = docs.title;
    let empty = CellArtifacts::default();
    let reference = docs.reference().map_or(&empty, |c| &c.artifacts);
    let (checkpoint_cell, steps_cell, insns_cell, convergence_cell, byte_parity_cell) =
        data_cells(reference);
    let config_cell = title
        .reference_key()
        .map_or_else(|| NO_DATA.to_string(), |k| k.label());

    assert_table_safe("title manifest field `content_id`", &title.content_id);
    assert_table_safe("title manifest field `display_name`", &title.display_name);
    assert_table_safe("title manifest field `developer`", &title.developer);
    assert_table_safe("title manifest field `engine`", &title.engine);
    assert_table_safe("the reference cell's label", &config_cell);

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

/// The five measurement columns for one cell's artifacts:
/// `(checkpoint, steps, insns, convergence, byte parity)`, each
/// [`NO_DATA`] when the file behind it is absent.
pub(super) fn data_cells(artifacts: &CellArtifacts) -> (String, String, String, String, String) {
    let (checkpoint, steps, insns) = match &artifacts.boot {
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
    let (convergence, byte_parity) = match &artifacts.cross {
        Some(c) => c.display_matrix_columns(),
        None => (NO_DATA.to_string(), NO_DATA.to_string()),
    };
    // These three quote a committed summary; no loader checks a
    // summary against the table's rules.
    assert_table_safe("the cell's checkpoint", &checkpoint);
    assert_table_safe("the cell's convergence", &convergence);
    assert_table_safe("the cell's byte parity", &byte_parity);
    (checkpoint, steps, insns, convergence, byte_parity)
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
