//! Title-manifest TOML parsing and eboot-candidate order validation.

use super::*;
use crate::game::manifest::test_fixtures::{FIRST_RSX_WRITE_TOML, PC_TOML, PROCESS_EXIT_TOML};

fn parse(text: &str) -> TitleManifest {
    TitleManifest::load_from_text(text, Path::new("test.toml")).unwrap()
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

#[test]
fn parses_each_distribution_variant() {
    for (token, expected) in [
        ("psn-hdd", Distribution::PsnHdd),
        ("retail-hdd", Distribution::RetailHdd),
        ("disc-iso", Distribution::DiscIso),
    ] {
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
host = "tools/rpcs3/dev_hdd0"

[[fs.mounts]]
prefix = "/app_home"
host = "tests/fixtures/flow_assets"
override_env = "CELLGOV_FLOW_APP_HOME"
"#;
    let m = parse(text);
    assert_eq!(m.mounts.len(), 2);
    assert_eq!(m.mounts[0].prefix, "/dev_hdd0");
    assert_eq!(m.mounts[0].host, "tools/rpcs3/dev_hdd0");
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

#[test]
fn cellgov_key_as_scalar_is_rejected() {
    let text = r#"
cellgov = "hello"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("scalar.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::Parse { .. }));
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

#[test]
fn unknown_fields_in_manifest_are_rejected() {
    let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]
short-name = "typo"

[checkpoint]
kind = "process-exit"
"#;
    let err = TitleManifest::load_from_text(text, Path::new("typo.toml")).expect_err("rejects");
    assert!(matches!(err, ManifestError::Parse { .. }));
}
