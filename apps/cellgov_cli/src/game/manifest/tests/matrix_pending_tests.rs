//! The `pending` marker: a cell that states why it cannot be measured
//! yet.

use std::path::Path;

use super::super::model::TitleManifest;
use super::derived_key;

/// The floor the fixture states.
const FLOOR: &str = "1.50";

fn manifest_with(rows: &str) -> String {
    format!(
        r#"
[title]
content_id = "NPAA00200"
short_name = "pending-fixture"
display_name = "Pending cell fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "psn-hdd"
system_ver = "{FLOOR}"

[checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

/// A row that repeats the derived cell with `pending = "<reason>"`.
fn pending_row(reason: &str) -> String {
    format!("\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\npending = \"{reason}\"\n")
}

fn origin() -> &'static Path {
    Path::new("title_manifests/pending-fixture.toml")
}

const REASON: &str = "the firmware is not obtainable";

#[test]
fn a_row_carries_the_reason_the_derived_cell_cannot_be_measured() {
    let text = manifest_with(&pending_row(REASON));
    let m = TitleManifest::load_from_text(&text, origin()).expect("manifest loads");
    assert_eq!(m.matrix.len(), 1);
    let cell = m.cell(&derived_key(FLOOR)).expect("the derived cell");
    assert_eq!(cell.pending.as_deref(), Some(REASON));
}

#[test]
fn a_cell_that_states_no_reason_is_not_pending() {
    let m = TitleManifest::load_from_text(&manifest_with(""), origin()).expect("manifest loads");
    assert_eq!(m.matrix[0].pending, None);
}

#[test]
fn a_row_beside_the_derived_cell_carries_its_own_reason() {
    let text = manifest_with(&format!(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\npending = \"{REASON}\"\n"
    ));
    let m = TitleManifest::load_from_text(&text, origin()).expect("manifest loads");
    assert_eq!(m.matrix[0].pending, None);
    assert_eq!(m.matrix[1].pending.as_deref(), Some(REASON));
}

#[test]
fn an_empty_reason_is_refused() {
    let err = TitleManifest::load_from_text(&manifest_with(&pending_row("   ")), origin())
        .expect_err("an empty reason names nothing")
        .to_string();
    assert!(err.contains("pending"), "{err}");
    assert!(err.contains("empty"), "{err}");
}

#[test]
fn a_reason_carrying_a_table_separator_is_refused() {
    let err = TitleManifest::load_from_text(
        &manifest_with(&pending_row("the loader fails | the boot ends early")),
        origin(),
    )
    .expect_err("a pipe ends the table cell the reason renders in")
    .to_string();
    assert!(err.contains("pending"), "{err}");
    assert!(err.contains("markdown table"), "{err}");
}

#[test]
fn a_reason_carrying_a_newline_is_refused() {
    let text = manifest_with(&format!(
        "
[[bench.matrix]]
fw = \"{FLOOR}\"
game_ver = \"base\"
pending = \"\"\"
the loader fails
the boot ends early\"\"\"
"
    ));
    let err = TitleManifest::load_from_text(&text, origin())
        .expect_err("a newline ends the table row the reason renders in")
        .to_string();
    assert!(err.contains("pending"), "{err}");
    assert!(err.contains("markdown table"), "{err}");
}

#[test]
fn a_reason_carrying_a_bare_carriage_return_is_refused() {
    let err = TitleManifest::load_from_text(
        &manifest_with(&pending_row("the loader fails\\rthe boot ends early")),
        origin(),
    )
    .expect_err("a bare carriage return ends the table row the reason renders in")
    .to_string();
    assert!(err.contains("pending"), "{err}");
    assert!(err.contains("markdown table"), "{err}");
}
