//! TOML -> [`TitleManifest`] translation. Owns [`ManifestError`] and
//! the second `impl TitleManifest` block carrying `load_from_path` /
//! `load_from_text`.

use std::path::{Path, PathBuf};

use super::checkpoint::{parse_pc_literal, CheckpointTrigger};
use super::model::{
    ContentEntry, ContentManifest, Distribution, GameSource, MountEntry, TitleManifest,
};
use super::schema::ManifestFile;

#[derive(Debug)]
pub enum ManifestError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    UnknownCheckpointKind {
        path: PathBuf,
        kind: String,
    },
    /// `kind = "pc"` without a value, or with a malformed value.
    BadCheckpointPc {
        path: PathBuf,
        detail: String,
    },
    DuplicateShortName {
        name: String,
        first: PathBuf,
        second: PathBuf,
        files_identical: bool,
    },
    DuplicateContentId {
        content_id: String,
        first: PathBuf,
        second: PathBuf,
        files_identical: bool,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "read {}: {source}", path.display())
            }
            Self::Parse { path, message } => {
                write!(f, "parse {}: {message}", path.display())
            }
            Self::UnknownCheckpointKind { path, kind } => write!(
                f,
                "{}: unknown checkpoint kind '{kind}' (accepted: \
                 process-exit, first-rsx-write, pc)",
                path.display()
            ),
            Self::BadCheckpointPc { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
            Self::DuplicateShortName {
                name,
                first,
                second,
                files_identical,
            } => {
                write!(
                    f,
                    "duplicate title short_name '{name}' in {} and {}",
                    first.display(),
                    second.display()
                )?;
                if *files_identical {
                    write!(f, " (files are byte-identical; one is likely a stray copy)")?;
                }
                Ok(())
            }
            Self::DuplicateContentId {
                content_id,
                first,
                second,
                files_identical,
            } => {
                write!(
                    f,
                    "duplicate title content_id '{content_id}' in {} and {}",
                    first.display(),
                    second.display()
                )?;
                if *files_identical {
                    write!(f, " (files are byte-identical; one is likely a stray copy)")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. }
            | Self::UnknownCheckpointKind { .. }
            | Self::BadCheckpointPc { .. }
            | Self::DuplicateShortName { .. }
            | Self::DuplicateContentId { .. } => None,
        }
    }
}

impl TitleManifest {
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::load_from_text(&text, path)
    }

    /// Accepts tables either at root level or under `[cellgov]`
    /// (microtests co-locate CellGov and RPCS3 config in one file).
    pub fn load_from_text(text: &str, origin: &Path) -> Result<Self, ManifestError> {
        let raw: toml::Value = toml::from_str(text).map_err(|e| ManifestError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })?;
        // A `cellgov` key selects the nested layout; scalar/array
        // forms or root-level tables alongside `[cellgov.*]` are errors.
        let file_value = if let Some(nested) = raw.get("cellgov") {
            if !nested.is_table() {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "`cellgov` key must be a table (the nested manifest layout); \
                              got a scalar or array"
                        .to_string(),
                });
            }
            if let Some(table) = raw.as_table() {
                let conflicting: Vec<&str> =
                    ["title", "checkpoint", "source", "rsx", "content", "fs"]
                        .iter()
                        .copied()
                        .filter(|k| table.contains_key(*k))
                        .collect();
                if !conflicting.is_empty() {
                    return Err(ManifestError::Parse {
                        path: origin.to_path_buf(),
                        message: format!(
                            "ambiguous layout: `[cellgov]` is present, but root-level \
                             manifest tables were also found ({}). Pick one layout.",
                            conflicting.join(", ")
                        ),
                    });
                }
            }
            nested.clone()
        } else {
            raw
        };
        let file: ManifestFile =
            file_value
                .try_into()
                .map_err(|e: toml::de::Error| ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: e.to_string(),
                })?;
        let checkpoint = match file.checkpoint.kind.as_str() {
            "process-exit" => CheckpointTrigger::ProcessExit,
            "first-rsx-write" => CheckpointTrigger::FirstRsxWrite,
            "pc" => {
                let raw =
                    file.checkpoint
                        .pc
                        .as_ref()
                        .ok_or_else(|| ManifestError::BadCheckpointPc {
                            path: origin.to_path_buf(),
                            detail: "checkpoint kind 'pc' requires a 'pc = \"0xADDR\"' value"
                                .to_string(),
                        })?;
                let parsed = parse_pc_literal(raw).map_err(|e| ManifestError::BadCheckpointPc {
                    path: origin.to_path_buf(),
                    detail: e.to_string(),
                })?;
                CheckpointTrigger::Pc(parsed)
            }
            other => {
                return Err(ManifestError::UnknownCheckpointKind {
                    path: origin.to_path_buf(),
                    kind: other.to_string(),
                })
            }
        };
        let source = match file.source.as_ref().map(|s| s.kind.as_str()) {
            Some("disc") => GameSource::Disc,
            Some("hdd") => GameSource::Hdd,
            Some(other) => {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: format!("unknown source kind '{other}' (accepted: disc, hdd)"),
                });
            }
            None => GameSource::Hdd,
        };
        let rsx_mirror = file.rsx.map(|r| r.mirror).unwrap_or(false);
        if rsx_mirror && matches!(checkpoint, CheckpointTrigger::FirstRsxWrite) {
            return Err(ManifestError::Parse {
                path: origin.to_path_buf(),
                message: "`[rsx] mirror = true` is incompatible with \
                          `checkpoint.kind = \"first-rsx-write\"`: the mirror \
                          makes the RSX region writable, so the put-pointer \
                          write that FirstRsxWrite watches for cannot fault."
                    .to_string(),
            });
        }
        let content = file.content.map(|c| ContentManifest {
            base: c.base,
            override_base_env: c.override_base_env,
            files: c
                .files
                .into_iter()
                .map(|f| ContentEntry {
                    guest_path: f.guest_path,
                    host_path: f.host_path,
                })
                .collect(),
        });
        let mounts: Vec<MountEntry> = file
            .fs
            .map(|f| f.mounts)
            .unwrap_or_default()
            .into_iter()
            .map(|m| MountEntry {
                prefix: m.prefix,
                host: m.host,
                override_env: m.override_env,
            })
            .collect();
        // Validate prefix shape at load time so the error carries
        // the manifest path (FsMountTable::add catches dup at boot
        // but without the source path).
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for m in &mounts {
            if !m.prefix.starts_with('/') {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: format!("[[fs.mounts]] prefix {:?} must start with '/'", m.prefix),
                });
            }
            if !seen.insert(m.prefix.as_str()) {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: format!(
                        "[[fs.mounts]] duplicate prefix {:?}; each prefix must \
                         be declared at most once",
                        m.prefix
                    ),
                });
            }
        }
        let distribution = match file.title.distribution.as_str() {
            "psn-hdd" => Distribution::PsnHdd,
            "retail-hdd" => Distribution::RetailHdd,
            "disc-iso" => Distribution::DiscIso,
            other => {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: format!(
                        "unknown distribution '{other}' (accepted: psn-hdd, retail-hdd, disc-iso)"
                    ),
                });
            }
        };
        Ok(TitleManifest {
            content_id: file.title.content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            year: file.title.year,
            developer: file.title.developer,
            engine: file.title.engine,
            distribution,
            checkpoint,
            source,
            rsx_mirror,
            content,
            mounts,
        })
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(m.eboot_candidates, vec!["EBOOT.elf", "EBOOT.BIN"]);
        assert_eq!(m.checkpoint, CheckpointTrigger::ProcessExit);
        assert_eq!(m.year, 2007);
        assert_eq!(m.developer, "test-developer");
        assert_eq!(m.engine, "test-engine");
        assert_eq!(m.distribution, Distribution::PsnHdd);
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
        let err =
            TitleManifest::load_from_text(text, Path::new("missing_base.toml")).expect_err("bad");
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
        let err =
            TitleManifest::load_from_text(text, Path::new("whatever.toml")).expect_err("rejects");
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
        let err =
            TitleManifest::load_from_text(text, Path::new("scalar.toml")).expect_err("rejects");
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
}
