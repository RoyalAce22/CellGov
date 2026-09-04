//! `[[bench.matrix]]` parsing -- cell defaults, the one-reference rule,
//! and the refusal shapes.

use std::path::Path;

use super::super::checkpoint::CheckpointTrigger;
use super::super::model::TitleManifest;
use super::{CellExpectation, ManifestError, BASE_GAME_VER};

/// A psn-hdd manifest whose `[[bench.matrix]]` rows are `rows`.
fn hdd_with(rows: &str) -> String {
    format!(
        r#"
[title]
content_id = "NPAA00100"
short_name = "cell-fixture"
display_name = "Cell matrix fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "psn-hdd"
bench_max_steps = 100_000_000

[checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

/// A firmware-exec manifest whose `[[bench.matrix]]` rows are `rows`.
fn firmware_exec_with(rows: &str) -> String {
    format!(
        r#"
[title]
content_id = "FWEXEC"
short_name = "fwexec-fixture"
display_name = "Firmware-exec cell fixture"
eboot_candidates = ["vsh.self"]
year = 2025
developer = "test-developer"
engine = "test-engine"
distribution = "firmware-exec"

[source]
kind = "firmware-exec"
path = "dev_flash/vsh/module"

[checkpoint]
kind = "process-exit"
{rows}
"#
    )
}

fn origin() -> &'static Path {
    Path::new("titles/cell-fixture.toml")
}

fn load(text: &str) -> TitleManifest {
    TitleManifest::load_from_text(text, origin()).expect("manifest loads")
}

fn refusal_err(text: &str) -> ManifestError {
    TitleManifest::load_from_text(text, origin()).expect_err("manifest is refused")
}

fn refusal(text: &str) -> String {
    refusal_err(text).to_string()
}

#[test]
fn a_manifest_with_no_bench_table_declares_no_cells() {
    assert!(load(&hdd_with("")).matrix.is_empty());
}

#[test]
fn an_empty_matrix_declares_no_cells() {
    let m = load(&hdd_with("\n[bench]\nmatrix = []\n"));
    assert!(m.matrix.is_empty());
    assert!(m.reference_cell().is_none());
}

#[test]
fn a_cell_defaults_to_the_frontier_expectation() {
    let m = load(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n",
    ));
    let cell = m.reference_cell().expect("the declared reference cell");
    assert_eq!(cell.key.fw, "4.91");
    assert_eq!(cell.key.game_ver.as_deref(), Some(BASE_GAME_VER));
    assert_eq!(cell.expect, CellExpectation::Frontier);
    assert_eq!(cell.bench_max_steps, None);
    assert_eq!(cell.checkpoint, None);
}

#[test]
fn a_cell_may_declare_the_probe_expectation() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
expect = "probe"
"#,
    ));
    assert_eq!(m.matrix[1].expect, CellExpectation::Probe);
    assert!(!m.matrix[1].reference);
}

#[test]
fn an_unknown_expectation_is_refused_and_names_the_accepted_ones() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\nexpect = \"maybe\"\n",
    ));
    assert!(err.contains("\"maybe\""), "{err}");
    assert!(err.contains("frontier") && err.contains("probe"), "{err}");
}

#[test]
fn cells_keep_their_declaration_order() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "3.55"
game_ver = "base"

[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true
"#,
    ));
    let order: Vec<&str> = m.matrix.iter().map(|c| c.key.fw.as_str()).collect();
    assert_eq!(order, ["3.55", "4.91"]);
}

#[test]
fn a_matrix_that_marks_no_reference_is_refused_naming_every_cell() {
    let err = refusal(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
"#,
    ));
    assert!(err.contains("marks none"), "{err}");
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("fw 3.55 x base"), "{err}");
}

#[test]
fn a_matrix_that_marks_two_references_is_refused_naming_both() {
    let err = refusal(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
reference = true
"#,
    ));
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("fw 3.55 x base"), "{err}");
    assert!(err.contains("exactly one"), "{err}");
}

#[test]
fn one_reference_among_several_cells_loads() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"

[[bench.matrix]]
fw = "4.91"
game_ver = "02.51"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
"#,
    ));
    assert_eq!(m.matrix.len(), 3);
    let cell = m.reference_cell().expect("the declared reference cell");
    assert_eq!(
        (cell.key.fw.as_str(), cell.key.game_ver.as_deref()),
        ("4.91", Some("02.51"))
    );
}

#[test]
fn an_unknown_key_in_a_row_is_refused() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\nbudget = 512\n",
    ));
    assert!(err.contains("unknown field"), "{err}");
    assert!(err.contains("budget"), "{err}");
}

#[test]
fn a_repeated_cell_is_refused() {
    let err = refusal(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "4.91"
game_ver = "base"
"#,
    ));
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("twice"), "{err}");
}

#[test]
fn a_row_with_no_game_ver_is_refused_for_a_title_with_a_version_axis() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\nreference = true\n",
    ));
    assert!(err.contains("states no game_ver"), "{err}");
    assert!(err.contains("4.91"), "{err}");
}

#[test]
fn a_game_ver_on_a_firmware_exec_title_is_refused() {
    let err = TitleManifest::load_from_text(
        &firmware_exec_with(
            "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n",
        ),
        origin(),
    )
    .expect_err("a firmware-shipped title has no game-version axis")
    .to_string();
    assert!(err.contains("does not apply"), "{err}");
    assert!(err.contains("game_ver"), "{err}");
    assert!(err.contains("\"base\""), "{err}");
}

#[test]
fn a_firmware_exec_matrix_is_one_row_per_firmware() {
    let m = TitleManifest::load_from_text(
        &firmware_exec_with(
            r#"
[[bench.matrix]]
fw = "4.91"
reference = true

[[bench.matrix]]
fw = "3.55"
expect = "probe"
"#,
        ),
        origin(),
    )
    .expect("manifest loads");
    assert_eq!(m.matrix.len(), 2);
    assert!(m.matrix.iter().all(|c| c.key.game_ver.is_none()));
    let cell = m.reference_cell().expect("the declared reference cell");
    assert_eq!(cell.key.fw, "4.91");
}

#[test]
fn two_firmware_exec_rows_naming_one_firmware_are_refused() {
    let err = TitleManifest::load_from_text(
        &firmware_exec_with(
            "\n[[bench.matrix]]\nfw = \"4.91\"\nreference = true\n\n[[bench.matrix]]\nfw = \"4.91\"\n",
        ),
        origin(),
    )
    .expect_err("one firmware is one cell")
    .to_string();
    assert!(err.contains("twice"), "{err}");
    assert!(err.contains("fw 4.91"), "{err}");
}

#[test]
fn a_fw_that_cannot_name_a_store_directory_is_refused() {
    for bad in ["", ".4.91", "4.91.", "4 91", "../escape"] {
        let err = refusal(&hdd_with(&format!(
            "\n[[bench.matrix]]\nfw = \"{bad}\"\ngame_ver = \"base\"\nreference = true\n"
        )));
        assert!(
            err.contains("fw:") && err.contains(&format!("{bad:?}")),
            "{bad:?} refused without naming fw and quoting the key: {err}"
        );
    }
}

#[test]
fn a_game_ver_that_cannot_name_a_store_directory_is_refused() {
    for bad in ["", ".02.51", "02.51.", "02 51", "../escape"] {
        let err = refusal(&hdd_with(&format!(
            "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"{bad}\"\nreference = true\n"
        )));
        assert!(
            err.contains("game_ver") && err.contains(&format!("{bad:?}")),
            "{bad:?} refused without naming game_ver and quoting the key: {err}"
        );
    }
}

#[test]
fn a_cell_overrides_the_title_level_cap_for_itself_alone() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
bench_max_steps = 250_000_000
"#,
    ));
    assert_eq!(m.bench_max_steps, Some(100_000_000));
    assert_eq!(m.matrix[0].bench_max_steps, None);
    assert_eq!(m.matrix[1].bench_max_steps, Some(250_000_000));
}

#[test]
fn a_cell_overrides_the_title_level_checkpoint_for_itself_alone() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
checkpoint = { kind = "pc", pc = "0x10381ce8" }
"#,
    ));
    assert_eq!(m.checkpoint, CheckpointTrigger::FirstRsxWrite);
    assert_eq!(m.matrix[0].checkpoint, None);
    assert_eq!(
        m.matrix[1].checkpoint,
        Some(CheckpointTrigger::Pc(0x1038_1ce8))
    );
}

#[test]
fn a_per_cell_checkpoint_is_refused_on_the_same_terms_as_the_title_level_one() {
    let unknown = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
         checkpoint = { kind = \"halt\" }\n",
    ));
    match &unknown {
        ManifestError::UnknownCheckpointKind { kind, .. } => assert_eq!(kind, "halt"),
        other => panic!("expected UnknownCheckpointKind, got {other:?}"),
    }
    let no_pc = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
         checkpoint = { kind = \"pc\" }\n",
    ));
    assert!(
        matches!(&no_pc, ManifestError::BadCheckpointPc { .. }),
        "{no_pc:?}"
    );
    assert!(no_pc.to_string().contains("requires a 'pc"), "{no_pc}");
    // The title-level parser reads the literal, so an unprefixed hex
    // string is refused here for the same reason.
    let bad_literal = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
         checkpoint = { kind = \"pc\", pc = \"1ce8\" }\n",
    ));
    assert!(
        matches!(&bad_literal, ManifestError::BadCheckpointPc { .. }),
        "{bad_literal:?}"
    );
    assert!(
        bad_literal.to_string().contains("not a decimal u64"),
        "{bad_literal}"
    );
}

#[test]
fn every_refusal_names_the_manifest_it_came_from() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\n",
    ));
    assert!(err.contains("cell-fixture.toml"), "{err}");
}

#[test]
fn a_bench_table_with_no_matrix_key_declares_no_cells() {
    assert!(load(&hdd_with("\n[bench]\n")).matrix.is_empty());
}

#[test]
fn an_explicitly_written_frontier_expectation_is_accepted() {
    let m = load(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
         expect = \"frontier\"\n",
    ));
    assert_eq!(m.matrix[0].expect, CellExpectation::Frontier);
}

#[test]
fn a_row_that_states_no_fw_is_refused_naming_the_missing_key() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\ngame_ver = \"base\"\nreference = true\n",
    ));
    assert!(err.contains("missing field `fw`"), "{err}");
}

#[test]
fn a_negative_per_cell_cap_is_refused() {
    let err = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
         bench_max_steps = -1\n",
    ));
    assert!(matches!(&err, ManifestError::Parse { .. }), "{err:?}");
}

/// The microtest layout, with every CellGov table under `[cellgov]`.
fn nested_cellgov_with(rows: &str) -> String {
    format!(
        r#"
[cellgov.title]
content_id = "NPAA00100"
short_name = "cell-fixture"
display_name = "Cell matrix fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

#[test]
fn the_nested_cellgov_layout_carries_the_matrix() {
    let m = load(&nested_cellgov_with(
        "\n[[cellgov.bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n",
    ));
    let cell = m.reference_cell().expect("the declared reference cell");
    assert_eq!(cell.key.fw, "4.91");
}

/// `bench` is a root-level manifest table, so the ambiguity check
/// covers it.
#[test]
fn a_root_matrix_beside_a_nested_cellgov_block_is_refused_as_ambiguous() {
    let text = format!(
        "{}\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n",
        nested_cellgov_with("")
    );
    let err = refusal(&text);
    assert!(err.contains("ambiguous layout"), "{err}");
    assert!(err.contains("bench"), "{err}");
}

/// Only a firmware-shipped source drops the game-version axis, so a
/// manifest-relative title names a game version like any other title.
#[test]
fn a_manifest_relative_title_declares_cells_on_the_game_version_axis() {
    let text = r#"
[title]
content_id = "mt"
short_name = "mt"
display_name = "Micro test"
eboot_candidates = ["mt.elf"]
year = 2025
developer = "test-developer"
engine = "test-engine"
distribution = "microtest"

[source]
kind = "manifest-relative"
path = "build"

[checkpoint]
kind = "process-exit"

[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true
"#;
    let m = load(text);
    assert_eq!(m.matrix.len(), 1);
    assert_eq!(m.matrix[0].key.game_ver.as_deref(), Some(BASE_GAME_VER));
    let err = refusal(&text.replace("game_ver = \"base\"\n", ""));
    assert!(err.contains("states no game_ver"), "{err}");
}

/// A mirrored title that stops at `process-exit`. The title-level
/// pairing refusal stays quiet, so only a cell override reaches
/// `first-rsx-write`.
fn mirrored_with(rows: &str) -> String {
    format!(
        r#"
[title]
content_id = "NPAA00101"
short_name = "mirror-fixture"
display_name = "Mirrored cell fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[rsx]
mirror = true
{rows}
"#
    )
}

#[test]
fn a_cell_cannot_override_its_way_into_the_checkpoint_the_mirror_makes_unreachable() {
    let err = TitleManifest::load_from_text(
        &mirrored_with(
            "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
             checkpoint = { kind = \"first-rsx-write\" }\n",
        ),
        origin(),
    )
    .expect_err("the mirror leaves the put-pointer write unable to fault")
    .to_string();
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("first-rsx-write"), "{err}");
    assert!(err.contains("mirror"), "{err}");
}

#[test]
fn a_mirrored_title_accepts_a_cell_overriding_to_a_reachable_checkpoint() {
    let m = TitleManifest::load_from_text(
        &mirrored_with(
            "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\nreference = true\n\
             checkpoint = { kind = \"pc\", pc = \"0x10381ce8\" }\n",
        ),
        origin(),
    )
    .expect("a pc checkpoint stays reachable under the mirror");
    assert_eq!(
        m.matrix[0].checkpoint,
        Some(CheckpointTrigger::Pc(0x1038_1ce8))
    );
}

#[test]
fn a_probe_cell_is_refused_as_the_reference() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nreference = true\nexpect = \"probe\"\n",
    ));
    assert!(err.contains("fw 3.55 x base"), "{err}");
    assert!(err.contains("probe"), "{err}");
    assert!(err.contains("reference = true"), "{err}");
}

#[test]
fn a_probe_cell_beside_a_frontier_reference_is_accepted() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "4.91"
game_ver = "base"
reference = true

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
expect = "probe"
"#,
    ));
    assert_eq!(
        m.reference_cell().expect("the reference cell").expect,
        CellExpectation::Frontier
    );
    assert_eq!(m.matrix[1].expect, CellExpectation::Probe);
}
