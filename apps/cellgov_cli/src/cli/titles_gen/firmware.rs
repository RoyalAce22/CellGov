//! `docs/firmware.md`: one row per declared cell of every title shipped
//! inside the firmware image.
//!
//! Such a title has no floor of its own -- its version axis is the
//! firmware axis -- so it has no headline row on the title index. Its
//! rows here are firmware versions, in the order its manifest declares
//! them.

use super::detail::detail_page_link;
use super::index::{assert_table_safe, coverage_counts, data_cells};
use super::load::TitleDocs;

const FIRMWARE_TEMPLATE: &str = include_str!("../templates/firmware.md.template");

/// Render the whole `docs/firmware.md` body from the firmware-shipped
/// titles in `docs`.
pub(crate) fn render(docs: &[TitleDocs<'_>]) -> String {
    let shipped: Vec<&TitleDocs<'_>> = docs.iter().filter(|d| d.ships_in_firmware()).collect();
    let rows: Vec<String> = shipped
        .iter()
        .flat_map(|d| d.cells.iter().map(|c| render_row(d, c)))
        .collect();
    super::super::fixture_gen::apply_subs(
        FIRMWARE_TEMPLATE,
        &[
            ("firmware_rows", &rows.join("\n")),
            ("coverage", &render_coverage(&shipped)),
        ],
    )
}

/// The page's one-line coverage count, over the firmware-shipped
/// titles alone.
fn render_coverage(shipped: &[&TitleDocs<'_>]) -> String {
    let (firmwares, declared, recorded) = coverage_counts(shipped.iter().copied());
    format!(
        "{} firmware-shipped title(s), {firmwares} firmware(s), {declared} declared cell(s), \
         {recorded} recorded.",
        shipped.len(),
    )
}

/// One markdown table row: one title at one firmware.
fn render_row(docs: &TitleDocs<'_>, cell: &super::load::LoadedCell) -> String {
    let title = docs.title;
    let (checkpoint, steps, insns, convergence, byte_parity) = data_cells(&cell.artifacts);
    let config = cell.key.label();
    assert_table_safe("title manifest field `content_id`", &title.content_id);
    assert_table_safe("title manifest field `display_name`", &title.display_name);
    assert_table_safe("the cell's label", &config);
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} |",
        detail_page_link(&title.content_id),
        title.display_name,
        config,
        checkpoint,
        steps,
        insns,
        convergence,
        byte_parity,
    )
}

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;
