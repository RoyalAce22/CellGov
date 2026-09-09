//! `[[bench.matrix]]` parsing -- the derived cell, row defaults, and the
//! refusal shapes.

use std::path::Path;

use super::super::checkpoint::CheckpointTrigger;
use super::super::model::TitleManifest;
use super::{derived_key, CellExpectation, CellKey, ManifestError, BASE_GAME_VER};

/// The floor every hdd fixture below states.
const FLOOR: &str = "4.91";

/// A psn-hdd manifest at [`FLOOR`] whose `[[bench.matrix]]` rows are
/// `rows`.
fn hdd_with(rows: &str) -> String {
    hdd_at(Some(FLOOR), rows)
}

/// A psn-hdd manifest whose `[title] system_ver` is `system_ver` and
/// whose `[[bench.matrix]]` rows are `rows`.
fn hdd_at(system_ver: Option<&str>, rows: &str) -> String {
    let system_ver = system_ver.map_or(String::new(), |v| format!("system_ver = \"{v}\"\n"));
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
{system_ver}
[checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

/// A firmware-exec manifest whose `[title]` ends in `title_tail` and
/// whose `[[bench.matrix]]` rows are `rows`.
fn firmware_exec_with(title_tail: &str, rows: &str) -> String {
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
{title_tail}
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
    Path::new("title_manifests/cell-fixture.toml")
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

fn floor_key() -> CellKey {
    derived_key(FLOOR)
}

// -- the derived cell --

#[test]
fn a_manifest_with_no_bench_table_declares_the_derived_cell_alone() {
    let m = load(&hdd_with(""));
    assert_eq!(m.matrix.len(), 1);
    let cell = &m.matrix[0];
    assert_eq!(cell.key, floor_key());
    assert_eq!(cell.key.game_ver.as_deref(), Some(BASE_GAME_VER));
    assert_eq!(cell.expect, CellExpectation::Frontier);
    assert_eq!(cell.bench_max_steps, None);
    assert_eq!(cell.checkpoint, None);
    assert_eq!(cell.pending, None);
    assert_eq!(m.reference_key(), Some(floor_key()));
}

#[test]
fn an_empty_matrix_declares_the_derived_cell_alone() {
    let m = load(&hdd_with("\n[bench]\nmatrix = []\n"));
    assert_eq!(m.matrix.len(), 1);
    assert_eq!(m.matrix[0].key, floor_key());
}

#[test]
fn a_bench_table_with_no_matrix_key_declares_the_derived_cell_alone() {
    assert_eq!(load(&hdd_with("\n[bench]\n")).matrix.len(), 1);
}

#[test]
fn a_title_with_a_param_sfo_and_no_system_ver_is_refused() {
    let err = refusal(&hdd_at(None, ""));
    assert!(err.contains("system_ver"), "{err}");
    assert!(err.contains("PS3_SYSTEM_VER"), "{err}");
    assert!(err.contains("required"), "{err}");
}

#[test]
fn a_system_ver_that_cannot_name_a_store_directory_is_refused() {
    for bad in ["", ".4.91", "4.91.", "4 91", "../escape", "01.5000/"] {
        let err = refusal(&hdd_at(Some(bad), ""));
        assert!(
            err.contains("system_ver") && err.contains(&format!("{bad:?}")),
            "{bad:?} refused without naming system_ver and quoting the key: {err}"
        );
    }
}

#[test]
fn a_system_ver_on_a_firmware_exec_title_is_refused() {
    let err =
        TitleManifest::load_from_text(&firmware_exec_with("system_ver = \"4.91\"\n", ""), origin())
            .expect_err("a firmware-shipped title has no PARAM.SFO to state a floor")
            .to_string();
    assert!(err.contains("system_ver"), "{err}");
    assert!(err.contains("\"4.91\""), "{err}");
    assert!(err.contains("shipped inside the firmware"), "{err}");
}

#[test]
fn a_system_ver_on_a_manifest_relative_title_is_refused() {
    let err = refusal(&manifest_relative_with("system_ver = \"4.91\"\n", ""));
    assert!(err.contains("system_ver"), "{err}");
    assert!(err.contains("beside its manifest"), "{err}");
}

// -- rows beside the derived cell --

#[test]
fn a_row_may_declare_the_probe_expectation() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "3.55"
game_ver = "base"
expect = "probe"
"#,
    ));
    assert_eq!(m.matrix.len(), 2);
    assert_eq!(m.matrix[0].expect, CellExpectation::Frontier);
    assert_eq!(m.matrix[1].expect, CellExpectation::Probe);
}

#[test]
fn an_unknown_expectation_is_refused_and_names_the_accepted_ones() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nexpect = \"maybe\"\n",
    ));
    assert!(err.contains("\"maybe\""), "{err}");
    assert!(err.contains("frontier") && err.contains("probe"), "{err}");
}

#[test]
fn rows_keep_their_declaration_order_behind_the_derived_cell() {
    let m = load(&hdd_with(
        r#"
[[bench.matrix]]
fw = "3.55"
game_ver = "base"

[[bench.matrix]]
fw = "2.76"
game_ver = "base"

[[bench.matrix]]
fw = "3.55"
game_ver = "02.51"
"#,
    ));
    let order: Vec<(&str, Option<&str>)> = m
        .matrix
        .iter()
        .map(|c| (c.key.fw.as_str(), c.key.game_ver.as_deref()))
        .collect();
    assert_eq!(
        order,
        [
            (FLOOR, Some(BASE_GAME_VER)),
            ("3.55", Some(BASE_GAME_VER)),
            ("2.76", Some(BASE_GAME_VER)),
            ("3.55", Some("02.51")),
        ]
    );
}

#[test]
fn an_explicitly_written_frontier_expectation_is_accepted_on_a_row() {
    let m = load(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nexpect = \"frontier\"\n",
    ));
    assert_eq!(m.matrix[1].expect, CellExpectation::Frontier);
}

#[test]
fn an_unknown_key_in_a_row_is_refused() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nbudget = 512\n",
    ));
    assert!(err.contains("unknown field"), "{err}");
    assert!(err.contains("budget"), "{err}");
}

#[test]
fn the_reference_flag_is_no_longer_a_key() {
    let err = refusal(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nreference = true\n",
    ));
    assert!(err.contains("unknown field"), "{err}");
    assert!(err.contains("reference"), "{err}");
}

#[test]
fn a_repeated_row_is_refused() {
    let err = refusal(&hdd_with(
        r#"
[[bench.matrix]]
fw = "3.55"
game_ver = "base"

[[bench.matrix]]
fw = "3.55"
game_ver = "base"
"#,
    ));
    assert!(err.contains("fw 3.55 x base"), "{err}");
    assert!(err.contains("twice"), "{err}");
}

#[test]
fn a_row_with_no_game_ver_is_refused_for_a_title_with_a_version_axis() {
    let err = refusal(&hdd_with("\n[[bench.matrix]]\nfw = \"3.55\"\n"));
    assert!(err.contains("states no game_ver"), "{err}");
    assert!(err.contains("3.55"), "{err}");
}

#[test]
fn a_fw_that_cannot_name_a_store_directory_is_refused() {
    for bad in ["", ".4.91", "4.91.", "4 91", "../escape"] {
        let err = refusal(&hdd_with(&format!(
            "\n[[bench.matrix]]\nfw = \"{bad}\"\ngame_ver = \"base\"\n"
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
            "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"{bad}\"\n"
        )));
        assert!(
            err.contains("game_ver") && err.contains(&format!("{bad:?}")),
            "{bad:?} refused without naming game_ver and quoting the key: {err}"
        );
    }
}

#[test]
fn a_row_that_states_no_fw_is_refused_naming_the_missing_key() {
    let err = refusal(&hdd_with("\n[[bench.matrix]]\ngame_ver = \"base\"\n"));
    assert!(err.contains("missing field `fw`"), "{err}");
}

#[test]
fn a_negative_per_cell_cap_is_refused() {
    let err = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\nbench_max_steps = -1\n",
    ));
    assert!(matches!(&err, ManifestError::Parse { .. }), "{err:?}");
}

#[test]
fn every_refusal_names_the_manifest_it_came_from() {
    let err = refusal(&hdd_with("\n[[bench.matrix]]\nfw = \"3.55\"\n"));
    assert!(err.contains("cell-fixture.toml"), "{err}");
}

// -- a row repeating the derived cell --

#[test]
fn a_row_repeating_the_derived_cell_with_nothing_to_add_is_refused() {
    for tail in ["", "expect = \"frontier\"\n"] {
        let err = refusal(&hdd_with(&format!(
            "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\n{tail}"
        )));
        assert!(err.contains("fw 4.91 x base"), "{err}");
        assert!(err.contains("system_ver"), "{err}");
        assert!(err.contains("adds nothing"), "{err}");
    }
}

#[test]
fn a_row_repeating_the_derived_cell_attaches_its_override_to_it() {
    let m = load(&hdd_with(&format!(
        "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\n\
         bench_max_steps = 250_000_000\n\
         checkpoint = {{ kind = \"pc\", pc = \"0x10381ce8\" }}\n"
    )));
    assert_eq!(
        m.matrix.len(),
        1,
        "the row attaches; it declares no second cell"
    );
    let cell = m.cell(&floor_key()).expect("the derived cell");
    assert_eq!(cell.bench_max_steps, Some(250_000_000));
    assert_eq!(cell.checkpoint, Some(CheckpointTrigger::Pc(0x1038_1ce8)));
    assert_eq!(cell.expect, CellExpectation::Frontier);
}

#[test]
fn a_row_repeating_the_derived_cell_attaches_its_pending_reason() {
    let m = load(&hdd_with(&format!(
        "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\n\
         pending = \"the boot faults before the checkpoint\"\n"
    )));
    assert_eq!(m.matrix.len(), 1);
    assert_eq!(
        m.matrix[0].pending.as_deref(),
        Some("the boot faults before the checkpoint")
    );
}

#[test]
fn a_row_repeating_the_derived_cell_as_a_probe_is_refused() {
    let err = refusal(&hdd_with(&format!(
        "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\nexpect = \"probe\"\n\
         bench_max_steps = 250_000_000\n"
    )));
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("probe"), "{err}");
    assert!(err.contains("another cell"), "{err}");
}

#[test]
fn two_rows_repeating_the_derived_cell_are_refused_as_a_repeat() {
    let err = refusal(&hdd_with(&format!(
        "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\nbench_max_steps = 1\n\
         \n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\nbench_max_steps = 2\n"
    )));
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("twice"), "{err}");
}

#[test]
fn a_row_at_the_floor_under_another_game_version_is_its_own_cell() {
    let m = load(&hdd_with(&format!(
        "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"02.51\"\n"
    )));
    assert_eq!(m.matrix.len(), 2);
    assert_eq!(m.matrix[1].key.game_ver.as_deref(), Some("02.51"));
}

// -- per-cell overrides --

#[test]
fn a_row_overrides_the_title_level_cap_for_itself_alone() {
    let m = load(&hdd_with(
        r#"
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
fn a_row_overrides_the_title_level_checkpoint_for_itself_alone() {
    let m = load(&hdd_with(
        r#"
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
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\n\
         checkpoint = { kind = \"halt\" }\n",
    ));
    match &unknown {
        ManifestError::UnknownCheckpointKind { kind, .. } => assert_eq!(kind, "halt"),
        other => panic!("expected UnknownCheckpointKind, got {other:?}"),
    }
    let no_pc = refusal_err(&hdd_with(
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\n\
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
        "\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\n\
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

// -- a title shipped inside the firmware --

#[test]
fn a_firmware_exec_matrix_is_one_row_per_firmware_and_derives_nothing() {
    let m = TitleManifest::load_from_text(
        &firmware_exec_with(
            "",
            r#"
[[bench.matrix]]
fw = "4.91"

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
    assert_eq!(m.matrix[0].key.fw, "4.91");
    assert_eq!(m.matrix[1].expect, CellExpectation::Probe);
    assert_eq!(m.system_ver, None);
    assert_eq!(m.reference_key(), None);
}

#[test]
fn a_firmware_exec_title_with_no_rows_declares_no_cells() {
    let m = TitleManifest::load_from_text(&firmware_exec_with("", ""), origin())
        .expect("manifest loads");
    assert!(m.matrix.is_empty());
}

#[test]
fn a_game_ver_on_a_firmware_exec_title_is_refused() {
    let err = TitleManifest::load_from_text(
        &firmware_exec_with(
            "",
            "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\n",
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
fn two_firmware_exec_rows_naming_one_firmware_are_refused() {
    let err = TitleManifest::load_from_text(
        &firmware_exec_with(
            "",
            "\n[[bench.matrix]]\nfw = \"4.91\"\n\n[[bench.matrix]]\nfw = \"4.91\"\n",
        ),
        origin(),
    )
    .expect_err("one firmware is one cell")
    .to_string();
    assert!(err.contains("twice"), "{err}");
    assert!(err.contains("fw 4.91"), "{err}");
}

// -- the nested layout --

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
system_ver = "{FLOOR}"

[cellgov.checkpoint]
kind = "first-rsx-write"
{rows}
"#
    )
}

#[test]
fn the_nested_cellgov_layout_carries_the_floor_and_the_matrix() {
    let m = load(&nested_cellgov_with(
        "\n[[cellgov.bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\n",
    ));
    assert_eq!(m.reference_key(), Some(floor_key()));
    assert_eq!(m.matrix.len(), 2);
    assert_eq!(m.matrix[1].key.fw, "3.55");
}

/// `bench` is a root-level manifest table, so the ambiguity check
/// covers it.
#[test]
fn a_root_matrix_beside_a_nested_cellgov_block_is_refused_as_ambiguous() {
    let text = format!(
        "{}\n[[bench.matrix]]\nfw = \"3.55\"\ngame_ver = \"base\"\n",
        nested_cellgov_with("")
    );
    let err = refusal(&text);
    assert!(err.contains("ambiguous layout"), "{err}");
    assert!(err.contains("bench"), "{err}");
}

// -- a title built beside its manifest --

/// A manifest-relative manifest whose `[title]` ends in `title_tail`
/// and whose `[[bench.matrix]]` rows are `rows`.
fn manifest_relative_with(title_tail: &str, rows: &str) -> String {
    format!(
        r#"
[title]
content_id = "mt"
short_name = "mt"
display_name = "Micro test"
eboot_candidates = ["mt.elf"]
year = 2025
developer = "test-developer"
engine = "test-engine"
distribution = "microtest"
{title_tail}
[source]
kind = "manifest-relative"
path = "build"

[checkpoint]
kind = "process-exit"
{rows}
"#
    )
}

/// Only a firmware-shipped source drops the game-version axis, so a
/// manifest-relative title names a game version like any other title.
/// It has no PARAM.SFO, so it derives no cell and needs no floor.
#[test]
fn a_manifest_relative_title_declares_cells_on_the_game_version_axis() {
    let m = load(&manifest_relative_with(
        "",
        "\n[[bench.matrix]]\nfw = \"4.91\"\ngame_ver = \"base\"\n",
    ));
    assert_eq!(m.matrix.len(), 1);
    assert_eq!(m.matrix[0].key.game_ver.as_deref(), Some(BASE_GAME_VER));
    assert_eq!(m.reference_key(), None);
    let err = refusal(&manifest_relative_with(
        "",
        "\n[[bench.matrix]]\nfw = \"4.91\"\n",
    ));
    assert!(err.contains("states no game_ver"), "{err}");
}

#[test]
fn a_manifest_relative_title_with_no_rows_declares_no_cells() {
    assert!(load(&manifest_relative_with("", "")).matrix.is_empty());
}

// -- the mirror --

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
system_ver = "{FLOOR}"

[checkpoint]
kind = "process-exit"

[rsx]
mirror = true
{rows}
"#
    )
}

#[test]
fn a_row_cannot_override_its_way_into_the_checkpoint_the_mirror_makes_unreachable() {
    let err = TitleManifest::load_from_text(
        &mirrored_with(&format!(
            "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\n\
             checkpoint = {{ kind = \"first-rsx-write\" }}\n"
        )),
        origin(),
    )
    .expect_err("the mirror leaves the put-pointer write unable to fault")
    .to_string();
    assert!(err.contains("fw 4.91 x base"), "{err}");
    assert!(err.contains("first-rsx-write"), "{err}");
    assert!(err.contains("mirror"), "{err}");
}

#[test]
fn a_mirrored_title_accepts_a_row_overriding_to_a_reachable_checkpoint() {
    let m = TitleManifest::load_from_text(
        &mirrored_with(&format!(
            "\n[[bench.matrix]]\nfw = \"{FLOOR}\"\ngame_ver = \"base\"\n\
             checkpoint = {{ kind = \"pc\", pc = \"0x10381ce8\" }}\n"
        )),
        origin(),
    )
    .expect("a pc checkpoint stays reachable under the mirror");
    assert_eq!(
        m.matrix[0].checkpoint,
        Some(CheckpointTrigger::Pc(0x1038_1ce8))
    );
}

// -- the floor's spelling, and the disc source --

mod floor_tests {
    use super::*;

    /// A disc-iso manifest whose `[title] system_ver` is `system_ver`
    /// and which declares no rows.
    fn disc_at(system_ver: Option<&str>) -> String {
        let system_ver = system_ver.map_or(String::new(), |v| format!("system_ver = \"{v}\"\n"));
        format!(
            r#"
[title]
content_id = "BCAA00100"
short_name = "cell-fixture"
display_name = "Disc cell fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "disc-iso"
{system_ver}
[source]
kind = "disc"

[checkpoint]
kind = "first-rsx-write"
"#
        )
    }

    /// `01.5000` passes the store's path-component check, so without
    /// its own refusal it would derive a cell at a firmware directory
    /// nothing installs.
    #[test]
    fn a_system_ver_spelled_the_way_param_sfo_spells_it_is_refused_naming_the_key() {
        for (raw, key) in [
            ("01.5000", "1.50"),
            ("04.9300", "4.93"),
            ("10.0100", "10.01"),
        ] {
            let err = refusal(&hdd_at(Some(raw), ""));
            assert!(err.contains("system_ver"), "{err}");
            assert!(err.contains(&format!("{raw:?}")), "{err}");
            assert!(err.contains(&format!("{key:?}")), "{err}");
            assert!(err.contains("PS3_SYSTEM_VER"), "{err}");
        }
    }

    #[test]
    fn a_disc_title_derives_its_cell_from_system_ver() {
        let m = load(&disc_at(Some(FLOOR)));
        assert_eq!(m.matrix.len(), 1);
        assert_eq!(m.matrix[0].key, floor_key());
        assert_eq!(m.reference_key(), Some(floor_key()));
    }

    #[test]
    fn a_disc_title_with_no_system_ver_is_refused() {
        let err = refusal(&disc_at(None));
        assert!(err.contains("system_ver"), "{err}");
        assert!(err.contains("required"), "{err}");
    }
}
