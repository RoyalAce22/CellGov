//! The `pending` marker: a cell that states why it cannot be measured
//! yet.

use std::path::Path;

use super::super::model::TitleManifest;

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

[checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

fn origin() -> &'static Path {
    Path::new("title_manifests/pending-fixture.toml")
}

const REASON: &str = "the firmware is not obtainable";

#[test]
fn a_cell_carries_the_reason_it_cannot_be_measured() {
    let text = manifest_with(&format!(
        r#"
[[bench.matrix]]
fw = "1.50"
game_ver = "base"
reference = true
pending = "{REASON}"
"#
    ));
    let m = TitleManifest::load_from_text(&text, origin()).expect("manifest loads");
    let cell = m.reference_cell().expect("one reference cell");
    assert_eq!(cell.pending.as_deref(), Some(REASON));
}

#[test]
fn a_cell_that_states_no_reason_is_not_pending() {
    let text = manifest_with(
        r#"
[[bench.matrix]]
fw = "4.93"
game_ver = "base"
reference = true
"#,
    );
    let m = TitleManifest::load_from_text(&text, origin()).expect("manifest loads");
    assert_eq!(
        m.reference_cell().expect("one reference cell").pending,
        None
    );
}

#[test]
fn an_empty_reason_is_refused() {
    let text = manifest_with(
        r#"
[[bench.matrix]]
fw = "1.50"
game_ver = "base"
reference = true
pending = "   "
"#,
    );
    let err = TitleManifest::load_from_text(&text, origin())
        .expect_err("an empty reason names nothing")
        .to_string();
    assert!(err.contains("pending"), "{err}");
    assert!(err.contains("empty"), "{err}");
}
