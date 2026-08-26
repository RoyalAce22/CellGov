//! Title-manifest TOML parsing and eboot-candidate order validation.

use super::*;
use crate::game::manifest::test_fixtures::{FIRST_RSX_WRITE_TOML, PC_TOML, PROCESS_EXIT_TOML};

fn parse(text: &str) -> TitleManifest {
    TitleManifest::load_from_text(text, Path::new("test.toml")).unwrap()
}

/// In scope for the microtest gates below: the manifest names at least
/// one table [`ManifestFile`] reads, in either layout `load_from_text`
/// accepts -- root level or under `[cellgov]` -- so a manifest cannot
/// leave the gate by switching layout. Keying on the whole table set
/// rather than on `title` alone keeps a half-declared manifest
/// (`[cellgov.source]`, no title) in: that is broken, not something to
/// skip. The compare-harness `[cellgov] scenario = ...` shape names
/// none of them and stays out.
///
/// Decided off the parsed TOML, not off line text: a header may be
/// spaced (`[ title ]`) or quoted, an inline table
/// (`scenario_args = { .. }`) is a table without being a header, and a
/// prose mention in a comment ("No [cellgov] section ...") must not
/// count.
///
/// # Errors
///
/// The manifest is not valid TOML. The gates report that rather than
/// skipping the file, which is what a text sniff would do.
fn declares_a_cellgov_title(text: &str) -> Result<bool, toml::de::Error> {
    let raw: toml::Value = toml::from_str(text)?;
    let names_a_manifest_table = |v: Option<&toml::Value>| {
        v.and_then(toml::Value::as_table)
            .is_some_and(|t| ROOT_TABLE_KEYS.iter().any(|k| t.contains_key(*k)))
    };
    Ok(names_a_manifest_table(Some(&raw)) || names_a_manifest_table(raw.get("cellgov")))
}

#[test]
fn the_microtest_gate_scope_follows_the_tables_not_the_line_text() {
    for (want, name, text) in [
        (true, "root layout", "[title]\nshort_name = \"x\"\n"),
        (true, "spaced header", "[ title ]\nshort_name = \"x\"\n"),
        (
            true,
            "nested layout",
            "[cellgov.title]\nshort_name = \"x\"\n",
        ),
        (
            true,
            "nested non-title table",
            "[cellgov.source]\nkind = \"hdd\"\n",
        ),
        (
            true,
            "nested array of tables",
            "[[cellgov.fs.mounts]]\nprefix = \"/\"\n",
        ),
        (
            false,
            "compare-harness scenario with an inline-table arg",
            "[cellgov]\nscenario = \"mailbox_send\"\nscenario_args = { messages = 1 }\n",
        ),
        (
            false,
            "prose mention only",
            "# No [cellgov] section, no [title] here.\n[rpcs3]\nbinary = \"b\"\n",
        ),
        (
            false,
            "rpcs3-only manifest",
            "[test]\nname = \"t\"\n\n[rpcs3]\nbinary = \"b\"\n",
        ),
    ] {
        assert_eq!(
            declares_a_cellgov_title(text).expect("fixture is valid TOML"),
            want,
            "{name}"
        );
    }
    declares_a_cellgov_title("not valid toml at all [[[")
        .expect_err("a broken manifest is reported, not skipped");
}

#[test]
fn parses_process_exit_manifest() {
    let m = parse(PROCESS_EXIT_TOML);
    assert_eq!(m.content_id, "NPAA00001");
    assert_eq!(m.short_name, "proc-exit-fixture");
    assert_eq!(m.eboot_candidates, vec!["EBOOT.BIN", "EBOOT.elf"]);
    assert_eq!(m.checkpoint, CheckpointTrigger::ProcessExit);
    assert_eq!(m.year, 2007);
    assert_eq!(m.developer, "test-developer");
    assert_eq!(m.engine, "test-engine");
    assert_eq!(m.distribution, Distribution::PsnHdd);
}

/// `[cellgov] scenario = ...` manifests are the compare-harness
/// shape and are out of scope.
#[test]
fn every_microtest_cellgov_manifest_parses_under_the_current_schema() {
    let micro_root = Path::new("../../tests/micro");
    assert!(
        micro_root.is_dir(),
        "tests/micro not found relative to the crate root"
    );
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(micro_root).expect("read tests/micro") {
        let manifest = entry.expect("dir entry").path().join("manifest.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).expect("manifest readable");
        match declares_a_cellgov_title(&text) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(err) => {
                failures.push(format!("{}: not valid TOML: {err}", manifest.display()));
                continue;
            }
        }
        checked += 1;
        if let Err(err) = TitleManifest::load_from_path(&manifest) {
            failures.push(format!("{}: {err}", manifest.display()));
        }
    }
    assert!(
        checked >= 12,
        "gate went vacuous: only {checked} CellGov-title manifests found under tests/micro"
    );
    assert!(
        failures.is_empty(),
        "stale microtest manifests:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rejects_eboot_candidates_with_elf_before_bin() {
    let bad = r#"
[title]
content_id = "X"
short_name = "x"
display_name = "x"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]
year = 2009
developer = "e"
engine = "e"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"
"#;
    match TitleManifest::load_from_text(bad, Path::new("bad.toml")) {
        Err(ManifestError::Parse { message, .. }) => {
            assert!(
                message.contains("EBOOT.elf before EBOOT.BIN"),
                "expected message to name the order violation; got {message:?}"
            );
        }
        other => panic!("expected Parse error, got {other:?}"),
    }
}

#[test]
fn elf_only_candidates_list_is_accepted() {
    let ok = r#"
[title]
content_id = "X"
short_name = "x"
display_name = "x"
eboot_candidates = ["EBOOT.elf"]
year = 2009
developer = "e"
engine = "e"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"
"#;
    let m = TitleManifest::load_from_text(ok, Path::new("synthetic.toml")).unwrap();
    assert_eq!(m.eboot_candidates, vec!["EBOOT.elf"]);
}

/// Driven off `VariantArray` rather than a hand-written list, so a
/// variant added to the enum cannot land with no wire-form coverage.
#[test]
fn parses_each_distribution_variant() {
    use strum::VariantArray as _;
    assert!(
        Distribution::VARIANTS.len() >= 5,
        "gate went vacuous: {} distribution variants",
        Distribution::VARIANTS.len()
    );
    for expected in Distribution::VARIANTS.iter().copied() {
        let token = expected.kebab_label();
        let text = format!(
            r#"
[title]
content_id = "X"
short_name = "x"
display_name = "x"
eboot_candidates = ["EBOOT.elf"]
year = 2009
developer = "e"
engine = "e"
distribution = "{token}"

[checkpoint]
kind = "process-exit"
"#
        );
        let m = TitleManifest::load_from_text(&text, Path::new("variant.toml")).unwrap();
        assert_eq!(m.distribution, expected, "token {token:?}");
    }
}

/// `from_kebab` scans the variant list and takes the first match, so a
/// shared wire form would resolve one variant and silently shadow the
/// other rather than being rejected anywhere.
#[test]
fn no_two_distributions_share_a_wire_form() {
    use strum::VariantArray as _;
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for v in Distribution::VARIANTS.iter().copied() {
        assert!(
            seen.insert(v.kebab_label()),
            "duplicate kebab label {:?} on {v:?}",
            v.kebab_label()
        );
    }
    assert_eq!(seen.len(), Distribution::VARIANTS.len());
}

#[test]
fn rejects_unknown_distribution() {
    let text = r#"
[title]
content_id = "X"
short_name = "x"
display_name = "x"
eboot_candidates = ["EBOOT.elf"]
year = 2009
developer = "e"
engine = "e"
distribution = "PSN-HDD"

[checkpoint]
kind = "process-exit"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("bad.toml"))
        .expect_err("uppercase variant must reject");
    match err {
        ManifestError::Parse { message, .. } => {
            assert!(
                message.contains("psn-hdd"),
                "diagnostic names allowed values: {message}"
            );
        }
        other => panic!("expected Parse, got {other:?}"),
    }
}

#[test]
fn rejects_missing_required_distribution_field() {
    let text = r#"
[title]
content_id = "X"
short_name = "x"
display_name = "x"
eboot_candidates = ["EBOOT.elf"]
year = 2009
developer = "e"

[checkpoint]
kind = "process-exit"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("missing.toml"))
        .expect_err("missing distribution must reject");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn parses_first_rsx_write_manifest() {
    let m = parse(FIRST_RSX_WRITE_TOML);
    assert_eq!(m.content_id, "NPAA00002");
    assert_eq!(m.short_name, "rsx-write-fixture");
    assert_eq!(m.checkpoint, CheckpointTrigger::FirstRsxWrite);
}

#[test]
fn parses_nested_cellgov_section() {
    let text = r#"
[test]
name = "dummy_microtest"

[rpcs3]
binary = "build/foo.elf"
decoder = "interpreter"

[cellgov.title]
content_id = "CG_TESTBED"
short_name = "testbed"
display_name = "Microtest bed"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"
"#;
    let m = parse(text);
    assert_eq!(m.content_id, "CG_TESTBED");
    assert_eq!(m.short_name, "testbed");
    assert_eq!(m.checkpoint, CheckpointTrigger::ProcessExit);
}

#[test]
fn rsx_mirror_defaults_to_false_when_table_absent() {
    let m = parse(PROCESS_EXIT_TOML);
    assert!(!m.rsx_mirror());
}

#[test]
fn content_block_absent_means_no_content_provider() {
    let m = parse(PROCESS_EXIT_TOML);
    assert!(m.content.is_none());
}

#[test]
fn parses_content_block_with_files() {
    let text = r#"
[title]
content_id = "NPAA77777"
short_name = "content-fixture"
display_name = "Content fixture"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[content]
base = "tests/fixtures/CONTENT_DIR"
files = [
{ guest_path = "/app_home/Data/Resources/first.xml", host_path = "first.xml" },
{ guest_path = "/app_home/Data/Local/Localization.xml", host_path = "Localization.xml" },
]
"#;
    let m = parse(text);
    let content = m.content.as_ref().expect("content present");
    assert_eq!(content.base, "tests/fixtures/CONTENT_DIR");
    assert!(
        content.override_base_env.is_none(),
        "override_base_env defaults to None when omitted",
    );
    assert_eq!(content.files.len(), 2);
    assert_eq!(
        content.files[0].guest_path,
        "/app_home/Data/Resources/first.xml",
    );
    assert_eq!(content.files[0].host_path, "first.xml");
}

#[test]
fn parses_content_block_with_override_base_env() {
    let text = r#"
[title]
content_id = "NPAA77779"
short_name = "override-fixture"
display_name = "Override fixture"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[content]
base = "tests/fixtures/synthetic"
override_base_env = "CELLGOV_NPAA77779_CONTENT_DIR"
files = [
{ guest_path = "/p", host_path = "h.bin" },
]
"#;
    let m = parse(text);
    let content = m.content.as_ref().expect("content present");
    assert_eq!(
        content.override_base_env.as_deref(),
        Some("CELLGOV_NPAA77779_CONTENT_DIR"),
    );
}

#[test]
fn parses_content_block_with_empty_files_array() {
    let text = r#"
[title]
content_id = "NPAA77778"
short_name = "empty-content"
display_name = "Empty content"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[content]
base = "."
files = []
"#;
    let m = parse(text);
    let content = m.content.as_ref().expect("content present");
    assert!(content.files.is_empty());
}

#[test]
fn content_block_missing_base_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[content]
files = []
"#;
    let err = TitleManifest::load_from_text(text, Path::new("missing_base.toml")).expect_err("bad");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn content_entry_with_unknown_field_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[content]
base = "."
files = [
{ guest_path = "/foo", "host-path" = "bar" },
]
"#;
    let err = TitleManifest::load_from_text(text, Path::new("typo.toml")).expect_err("bad");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn parses_content_block_from_nested_cellgov_section() {
    let text = r#"
[cellgov.title]
content_id = "CG_CONT"
short_name = "cgcontent"
display_name = "CG content"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"

[cellgov.content]
base = "fx"
files = [
{ guest_path = "/p", host_path = "h" },
]
"#;
    let m = parse(text);
    let content = m.content.as_ref().expect("nested content present");
    assert_eq!(content.base, "fx");
    assert_eq!(content.files.len(), 1);
}

#[test]
fn parses_rsx_mirror_true_from_root_table() {
    let text = r#"
[title]
content_id = "NPAA99999"
short_name = "mirror-fixture"
display_name = "Mirror fixture"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[rsx]
mirror = true
"#;
    let m = parse(text);
    assert!(m.rsx_mirror());
}

#[test]
fn parses_rsx_mirror_true_from_nested_cellgov_section() {
    let text = r#"
[cellgov.title]
content_id = "CG_MIRROR"
short_name = "cgmirror"
display_name = "CG mirror"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"

[cellgov.rsx]
mirror = true
"#;
    let m = parse(text);
    assert!(m.rsx_mirror());
}

#[test]
fn rsx_mirror_with_first_rsx_write_checkpoint_is_rejected() {
    let text = r#"
[title]
content_id = "NPAA88888"
short_name = "conflict"
display_name = "Conflict"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "first-rsx-write"

[rsx]
mirror = true
"#;
    let err = TitleManifest::load_from_text(text, Path::new("conflict.toml"))
        .expect_err("must reject incompatible combination");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn fs_mounts_block_absent_means_empty_mount_list() {
    let m = parse(PROCESS_EXIT_TOML);
    assert!(m.mounts.is_empty());
}

#[test]
fn parses_fs_mounts_array_in_declaration_order() {
    let text = r#"
[title]
content_id = "NPAA66666"
short_name = "mounts-fixture"
display_name = "Mounts fixture"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[[fs.mounts]]
prefix = "/dev_hdd0"
host = "vfs/dev_hdd0"

[[fs.mounts]]
prefix = "/app_home"
host = "tests/fixtures/flow_assets"
override_env = "CELLGOV_FLOW_APP_HOME"
"#;
    let m = parse(text);
    assert_eq!(m.mounts.len(), 2);
    assert_eq!(m.mounts[0].prefix, "/dev_hdd0");
    assert_eq!(m.mounts[0].host, "vfs/dev_hdd0");
    assert!(m.mounts[0].override_env.is_none());
    assert_eq!(m.mounts[1].prefix, "/app_home");
    assert_eq!(m.mounts[1].host, "tests/fixtures/flow_assets");
    assert_eq!(
        m.mounts[1].override_env.as_deref(),
        Some("CELLGOV_FLOW_APP_HOME"),
    );
}

#[test]
fn parses_fs_mounts_from_nested_cellgov_section() {
    let text = r#"
[cellgov.title]
content_id = "CG_MOUNTS"
short_name = "cgmounts"
display_name = "CG mounts"
eboot_candidates = ["EBOOT.elf"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"

[[cellgov.fs.mounts]]
prefix = "/app_home"
host = "fx"
"#;
    let m = parse(text);
    assert_eq!(m.mounts.len(), 1);
    assert_eq!(m.mounts[0].prefix, "/app_home");
}

#[test]
fn fs_mounts_prefix_without_leading_slash_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[[fs.mounts]]
prefix = "app_home"
host = "fx"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("bad.toml"))
        .expect_err("non-rooted prefix must reject");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn fs_mounts_duplicate_prefix_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[[fs.mounts]]
prefix = "/app_home"
host = "fx1"

[[fs.mounts]]
prefix = "/app_home"
host = "fx2"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("dup.toml"))
        .expect_err("duplicate prefix must reject");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn fs_mounts_unknown_field_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[[fs.mounts]]
prefix = "/app_home"
host_path = "fx"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("typo.toml"))
        .expect_err("unknown field must reject");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn parses_pc_manifest() {
    let m = parse(PC_TOML);
    assert_eq!(m.checkpoint, CheckpointTrigger::Pc(0x10381ce8));
}

#[test]
fn pc_kind_without_value_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "pc"
"#;
    let err =
        TitleManifest::load_from_text(text, Path::new("pc_missing.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::BadCheckpointPc { .. }));
}

#[test]
fn unknown_checkpoint_kind_is_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "whatever"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("whatever.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::UnknownCheckpointKind { .. }));
}

#[test]
fn malformed_toml_is_rejected() {
    let text = "not valid toml at all [[[";
    let err = TitleManifest::load_from_text(text, Path::new("bad.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

#[test]
fn pc_manifest_accepts_decimal_literal() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "pc"
pc = "256"
"#;
    let m = TitleManifest::load_from_text(text, Path::new("dec.toml")).unwrap();
    assert_eq!(m.checkpoint, CheckpointTrigger::Pc(256));
}

#[test]
fn pc_manifest_rejects_unprefixed_hex_letters() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "pc"
pc = "1ce8"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("bad.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::BadCheckpointPc { .. }));
}

/// Deleting the `is_table` guard would still reject this, via serde's
/// generic "invalid type" -- so the assertion has to reach the message,
/// or it passes without the named refusal existing.
#[test]
fn cellgov_key_as_scalar_is_rejected() {
    let text = r#"
cellgov = "hello"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("scalar.toml")).expect_err("rejects");
    match &err {
        ManifestError::Parse { message, .. } => assert!(
            message.contains("must be a table"),
            "message should name the layout it violates: {message}"
        ),
        other => panic!("expected Parse, got {other:?}"),
    }
}

/// The nested layout reads `[cellgov]` and ignores the root, so any
/// table [`ManifestFile`] accepts must appear in the loader's
/// conflict list -- one missing there is silently dropped instead of
/// raising the ambiguity refusal.
#[test]
fn root_table_keys_cover_every_manifest_table() {
    let err = toml::from_str::<ManifestFile>("this_is_not_a_manifest_table = 1")
        .expect_err("deny_unknown_fields rejects and enumerates the accepted tables")
        .to_string();
    let (_, listed) = err
        .split_once("expected one of ")
        .unwrap_or_else(|| panic!("serde no longer enumerates the accepted tables: {err}"));
    let accepted: Vec<&str> = listed
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        accepted.len(),
        ROOT_TABLE_KEYS.len(),
        "ManifestFile accepts {accepted:?}, conflict list is {ROOT_TABLE_KEYS:?}"
    );
    for key in accepted {
        assert!(
            ROOT_TABLE_KEYS.contains(&key),
            "root table {key:?} is missing from the ambiguity conflict list \
             {ROOT_TABLE_KEYS:?}; a nested-layout manifest would drop it silently"
        );
    }
}

/// Each conflicting root table is named, not just the first one found.
#[test]
fn the_ambiguous_layout_refusal_names_every_root_table_it_found() {
    let text = r#"
[title]
content_id = "root"
short_name = "root"
display_name = "root"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[rsx]
mirror = true

[cellgov.title]
content_id = "nested"
short_name = "nested"
display_name = "nested"
eboot_candidates = ["y"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("both.toml")).expect_err("rejects");
    let msg = err.to_string();
    for key in ["title", "checkpoint", "rsx"] {
        assert!(msg.contains(key), "refusal should name {key:?}: {msg}");
    }
    assert!(
        !msg.contains("source"),
        "refusal should name only the tables actually present: {msg}"
    );
}

#[test]
fn cellgov_nested_with_root_tables_is_rejected() {
    let text = r#"
[title]
content_id = "root"
short_name = "root"
display_name = "root"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"

[cellgov.title]
content_id = "nested"
short_name = "nested"
display_name = "nested"
eboot_candidates = ["y"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[cellgov.checkpoint]
kind = "process-exit"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("both.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::Parse { .. }));
}

/// The manifest is otherwise complete, so only the typo can be what
/// rejects it -- an incomplete fixture would reject on the missing
/// fields and never reach `deny_unknown_fields`.
#[test]
fn unknown_fields_in_manifest_are_rejected() {
    let complete = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
year = 2007
developer = "test"
engine = "test-engine"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"
"#;
    TitleManifest::load_from_text(complete, Path::new("ok.toml"))
        .expect("the fixture without the typo must load");
    let text = complete.replace(
        "short_name = \"x\"",
        "short_name = \"x\"\nshort-name = \"typo\"",
    );
    let err = TitleManifest::load_from_text(&text, Path::new("typo.toml")).expect_err("rejects");
    match &err {
        ManifestError::Parse { message, .. } => assert!(
            message.contains("short-name"),
            "message should name the unknown key: {message}"
        ),
        other => panic!("expected Parse, got {other:?}"),
    }
}

const FIRMWARE_EXEC_TOML: &str = r#"
[title]
content_id = "FWX"
short_name = "fwx"
display_name = "Firmware exec fixture"
eboot_candidates = ["vsh.self"]
year = 2025
developer = "test-developer"
engine = "test-engine"
distribution = "firmware-exec"

[source]
kind = "firmware-exec"
path = "firmware/vsh/module"

[checkpoint]
kind = "process-exit"
"#;

#[test]
fn firmware_exec_source_carries_its_directory() {
    let m = TitleManifest::load_from_text(FIRMWARE_EXEC_TOML, Path::new("fwx.toml"))
        .expect("firmware-exec manifest loads");
    assert_eq!(m.distribution, Distribution::FirmwareExec);
    assert_eq!(
        m.source,
        GameSource::FirmwareExec {
            dir: PathBuf::from("firmware/vsh/module")
        }
    );
    assert_eq!(m.rap_filename, None);
    assert_eq!(m.content, None);
    assert!(m.mounts.is_empty());
}

#[test]
fn firmware_exec_source_without_path_is_rejected() {
    let text = FIRMWARE_EXEC_TOML.replace(
        "path = \"firmware/vsh/module\"
",
        "",
    );
    let err = TitleManifest::load_from_text(&text, Path::new("fwx.toml"))
        .expect_err("firmware-exec needs a path");
    let msg = err.to_string();
    assert!(
        msg.contains("firmware-exec") && msg.contains("path"),
        "message should name the missing key: {msg}"
    );
}

#[test]
fn source_path_on_a_game_kind_is_rejected() {
    let text = FIRMWARE_EXEC_TOML.replace("kind = \"firmware-exec\"", "kind = \"hdd\"");
    let err = TitleManifest::load_from_text(&text, Path::new("fwx.toml"))
        .expect_err("path is firmware-exec only");
    assert!(
        err.to_string().contains("firmware-exec"),
        "message should say which kind accepts path: {err}"
    );
}

#[test]
fn unknown_source_kind_lists_every_accepted_kind() {
    let text = FIRMWARE_EXEC_TOML.replace("kind = \"firmware-exec\"", "kind = \"bluray\"");
    let err = TitleManifest::load_from_text(&text, Path::new("fwx.toml"))
        .expect_err("unknown kind rejects");
    let msg = err.to_string();
    for kind in ["disc", "hdd", "firmware-exec", "manifest-relative"] {
        assert!(
            msg.contains(kind),
            "accepted-kind list omits {kind:?}: {msg}"
        );
    }
}

#[test]
fn committed_vsh_manifest_declares_no_game_requirements() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root two levels up");
    let path = root.join("titles/VSH.toml");
    let m = TitleManifest::load_from_path(&path).expect("committed vsh manifest loads");
    assert_eq!(m.short_name, "vsh");
    assert_eq!(m.distribution, Distribution::FirmwareExec);
    assert!(matches!(m.source, GameSource::FirmwareExec { .. }));
    assert_eq!(m.rap_filename, None, "vsh.self is CoreOS-keyed, not NPDRM");
    assert_eq!(m.content, None);
    assert!(m.mounts.is_empty());
}

const MANIFEST_RELATIVE_TOML: &str = r#"
[title]
short_name = "atomicspin"
display_name = "CellGov PPU atomic spinlock microtest"
eboot_candidates = ["ppu_atomic_spinlock.elf"]
year = 2026
developer = "CellGov"
engine = "microtest"
distribution = "microtest"

[source]
kind = "manifest-relative"
path = "build"

[checkpoint]
kind = "process-exit"
"#;

#[test]
fn manifest_relative_source_resolves_against_the_manifests_own_directory() {
    let origin = Path::new("tests/micro/ppu_atomic_spinlock/manifest.toml");
    let m = TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, origin)
        .expect("manifest-relative manifest loads");
    assert_eq!(m.distribution, Distribution::Microtest);
    assert_eq!(
        m.source,
        GameSource::ManifestRelative {
            dir: PathBuf::from("tests/micro/ppu_atomic_spinlock").join("build")
        }
    );
}

/// What separates this kind from `firmware-exec`: one declared path
/// under two manifests names two directories, so the reference does
/// not depend on the process cwd.
#[test]
fn manifest_relative_path_is_not_taken_relative_to_the_process_cwd() {
    let here = TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, Path::new("a/manifest.toml"))
        .expect("loads")
        .source;
    let there = TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, Path::new("b/manifest.toml"))
        .expect("loads")
        .source;
    assert_ne!(here, there);
    assert_eq!(
        here,
        GameSource::ManifestRelative {
            dir: PathBuf::from("a").join("build")
        }
    );
}

#[test]
fn manifest_relative_source_without_path_is_rejected() {
    let text = MANIFEST_RELATIVE_TOML.replace("path = \"build\"\n", "");
    let err = TitleManifest::load_from_text(&text, Path::new("m.toml"))
        .expect_err("manifest-relative needs a path");
    let msg = err.to_string();
    assert!(
        msg.contains("manifest-relative") && msg.contains("path"),
        "message should name the missing key: {msg}"
    );
}

/// `Path::join` treats the empty path as a no-op, so an empty `path`
/// would leave the executable to be looked up against whatever base
/// was there rather than a directory the manifest declared.
#[test]
fn an_empty_source_path_is_rejected_for_both_path_bearing_kinds() {
    for (kind, toml) in [
        ("manifest-relative", MANIFEST_RELATIVE_TOML.to_string()),
        (
            "firmware-exec",
            MANIFEST_RELATIVE_TOML
                .replace("kind = \"manifest-relative\"", "kind = \"firmware-exec\"")
                .replace("[title]\n", "[title]\ncontent_id = \"X\"\n"),
        ),
    ] {
        let text = toml.replace("path = \"build\"", "path = \"\"");
        let err = TitleManifest::load_from_text(&text, Path::new("micro/mt/manifest.toml"))
            .expect_err("an empty path names no directory");
        let msg = err.to_string();
        assert!(
            msg.contains(kind) && msg.contains("empty"),
            "{kind}: message should name the kind and the empty path: {msg}"
        );
    }
}

/// `Path::join` drops the base when the joined path is rooted or
/// carries a drive prefix, so such a path would silently stop being
/// manifest-relative. Each form is first shown to actually discard the
/// base on this host, so the refusal list cannot go vacuous by naming a
/// form that joins harmlessly.
#[test]
fn a_rooted_manifest_relative_path_is_rejected_rather_than_silently_replacing_the_base() {
    // `/abs/build` is rooted on every host. The backslash and
    // drive-prefix forms are only path syntax on Windows; elsewhere
    // they are ordinary filename characters that join harmlessly, and
    // the loader is right to take them.
    let mut forms: Vec<&str> = vec!["/abs/build"];
    if cfg!(windows) {
        // `C:build` is drive-relative, not absolute: `is_absolute` is
        // false for it, yet `join` still replaces the base.
        forms.extend(["\\abs\\build", "C:\\abs\\build", "C:build"]);
    }
    let base = Path::new("micro/mt");
    for rooted in forms {
        assert!(
            !base.join(rooted).starts_with(base),
            "{rooted:?} does not discard its base here; the refusal would be vacuous"
        );
        let text =
            MANIFEST_RELATIVE_TOML.replace("path = \"build\"", &format!("path = {rooted:?}"));
        let err = TitleManifest::load_from_text(&text, Path::new("micro/mt/manifest.toml"))
            .expect_err("a rooted path is not manifest-relative");
        let msg = err.to_string();
        assert!(
            msg.contains("manifest-relative") && msg.contains("rooted"),
            "{rooted:?}: message should name the kind it violates: {msg}"
        );
    }
    // The converse: an ordinary relative path keeps the base, and the
    // loader takes it.
    assert!(base.join("build").starts_with(base));
    TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, Path::new("micro/mt/manifest.toml"))
        .expect("a plain relative path is what the kind is for");
}

/// A `firmware-exec` path is documented to resolve against the process
/// cwd, so an absolute host directory is the intended form there and
/// must not inherit the manifest-relative refusal.
#[test]
fn a_rooted_firmware_exec_path_is_accepted() {
    let abs = if cfg!(windows) {
        "C:/fw/vsh"
    } else {
        "/fw/vsh"
    };
    let text = FIRMWARE_EXEC_TOML.replace("firmware/vsh/module", abs);
    let m = TitleManifest::load_from_text(&text, Path::new("fwx.toml"))
        .expect("firmware-exec takes a host-absolute directory");
    assert_eq!(
        m.source,
        GameSource::FirmwareExec {
            dir: PathBuf::from(abs)
        }
    );
}

#[test]
fn manifest_relative_title_takes_its_content_id_from_its_directory() {
    let origin = Path::new("tests/micro/ppu_atomic_spinlock/manifest.toml");
    let m = TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, origin).expect("loads");
    assert_eq!(m.content_id, "ppu_atomic_spinlock");
}

#[test]
fn a_stated_content_id_still_wins_for_a_manifest_relative_title() {
    let text = MANIFEST_RELATIVE_TOML.replace("[title]\n", "[title]\ncontent_id = \"STATED\"\n");
    let origin = Path::new("tests/micro/ppu_atomic_spinlock/manifest.toml");
    let m = TitleManifest::load_from_text(&text, origin).expect("loads");
    assert_eq!(m.content_id, "STATED");
}

#[test]
fn manifest_relative_title_with_no_parent_directory_is_rejected() {
    let err = TitleManifest::load_from_text(MANIFEST_RELATIVE_TOML, Path::new("manifest.toml"))
        .expect_err("nothing to derive an identity from");
    assert!(
        err.to_string().contains("content_id"),
        "message should name the field it could not fill: {err}"
    );
}

#[test]
fn an_hdd_title_without_content_id_is_rejected() {
    let text = MANIFEST_RELATIVE_TOML.replace(
        "kind = \"manifest-relative\"\npath = \"build\"\n",
        "kind = \"hdd\"\n",
    );
    let err = TitleManifest::load_from_text(&text, Path::new("micro/m.toml"))
        .expect_err("only a manifest-relative title may omit content_id");
    let msg = err.to_string();
    assert!(
        msg.contains("content_id") && msg.contains("manifest-relative"),
        "message should say who may omit it: {msg}"
    );
}

/// Every microtest resolves its executable beside its own manifest,
/// under the filename `build.sh` writes -- no staging copy into the
/// VFS, no fabricated retail identity.
#[test]
fn every_microtest_manifest_boots_from_its_own_build_dir() {
    let micro_root = Path::new("../../tests/micro");
    let mut checked = 0usize;
    let mut problems = Vec::new();
    for entry in std::fs::read_dir(micro_root).expect("read tests/micro") {
        let dir = entry.expect("dir entry").path();
        let manifest = dir.join("manifest.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).expect("manifest readable");
        match declares_a_cellgov_title(&text) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(err) => {
                problems.push(format!("{}: not valid TOML: {err}", manifest.display()));
                continue;
            }
        }
        checked += 1;
        let name = manifest.display().to_string();
        let m = match TitleManifest::load_from_path(&manifest) {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let want = GameSource::ManifestRelative {
            dir: dir.join("build"),
        };
        if m.source != want {
            problems.push(format!(
                "{name}: source is {:?}, want its own build/",
                m.source
            ));
        }
        if m.distribution != Distribution::Microtest {
            problems.push(format!("{name}: distribution is {:?}", m.distribution));
        }
        if m.eboot_candidates.iter().any(|c| c.starts_with("EBOOT.")) {
            problems.push(format!(
                "{name}: eboot_candidates {:?} name a staged copy, not the built artifact",
                m.eboot_candidates
            ));
        }
        if text
            .lines()
            .any(|l| l.trim_start().starts_with("content_id"))
        {
            problems.push(format!("{name}: declares a content_id it does not need"));
        }
        if text.contains("vfs/dev_hdd0") {
            problems.push(format!(
                "{name}: header still documents a copy into the VFS"
            ));
        }
    }
    assert!(
        checked >= 12,
        "gate went vacuous: only {checked} CellGov-title manifests found under tests/micro"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
