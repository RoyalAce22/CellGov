//! Title registry driven by TOML manifests under `docs/titles/`.
//!
//! Every PS3 title the CLI's game harness knows about corresponds
//! to one TOML file named after its PSN content id (for example,
//! `docs/titles/NPUA80068.toml` for Super Stardust HD). The file
//! carries the four facts the harness cares about: content id,
//! short name, display name, and the main-executable candidate
//! list; plus the default boot checkpoint. Adding a new title
//! that fits the existing checkpoint kinds and the standard PS3
//! VFS layout is a single-file commit -- no Rust change.
//!
//! Title metadata lives only in `cellgov_cli`; no library crate
//! below knows titles exist. Downstream importers that pull
//! `cellgov_ppu` or `cellgov_compare` do not transitively pull a
//! registry of named games.

use std::path::{Path, PathBuf};

/// One title's manifest as loaded from `docs/titles/<content-id>.toml`.
///
/// Call sites receive a `&TitleManifest` instead of the former
/// `Title` enum; the field surface mirrors what the old enum's
/// methods returned so call-site migration is mostly mechanical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleManifest {
    /// PSN content id and the directory name under
    /// `/dev_hdd0/game/`. Stable per title; primary lookup key.
    pub content_id: String,
    /// Short CLI name for `--title <name>` and diagnostics
    /// (e.g. `"flow"`, `"sshd"`). Must be unique across the
    /// registry; the loader rejects duplicates.
    pub short_name: String,
    /// Human-readable label for log lines and help text.
    pub display_name: String,
    /// Main-executable filenames to try inside the title's
    /// `USRDIR/`, in priority order. Decrypted ELFs come first
    /// so encrypted SELFs are only considered when no decrypted
    /// form is available.
    pub eboot_candidates: Vec<String>,
    /// Boot checkpoint default for this title. The step loop
    /// stops at this point; the CLI's `--checkpoint` flag
    /// overrides per run.
    pub checkpoint: CheckpointTrigger,
}

/// Stop condition for a boot. The harness picks a default from
/// the title's manifest; `--checkpoint` overrides per run. Only
/// selectable via the CLI; no title uses `Pc` as its built-in
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointTrigger {
    /// Stop when the guest calls `sys_process_exit`.
    ProcessExit,
    /// Stop when the PPU attempts its first write into the RSX
    /// region. In CellGov that write faults with
    /// `MemError::ReservedWrite { region: "rsx", .. }`; the
    /// harness treats the fault as "checkpoint reached", not
    /// "boot broken".
    FirstRsxWrite,
    /// Stop the first time a step retires at the given guest PC.
    /// Only selectable via `--checkpoint pc=0xADDR`.
    Pc(u64),
}

impl CheckpointTrigger {
    /// Parse a `--checkpoint <kind>` value. Accepts
    /// `process-exit`, `first-rsx-write`, and `pc=0xHEX` (or
    /// `pc=DECIMAL`).
    pub fn parse_cli_value(value: &str) -> Result<Self, String> {
        match value {
            "process-exit" => Ok(Self::ProcessExit),
            "first-rsx-write" => Ok(Self::FirstRsxWrite),
            _ => {
                if let Some(rest) = value.strip_prefix("pc=") {
                    let hex = rest.trim_start_matches("0x").trim_start_matches("0X");
                    let parsed = if rest.starts_with("0x") || rest.starts_with("0X") {
                        u64::from_str_radix(hex, 16)
                    } else {
                        rest.parse::<u64>()
                            .or_else(|_| u64::from_str_radix(hex, 16))
                    };
                    parsed
                        .map(Self::Pc)
                        .map_err(|_| format!("checkpoint pc value '{rest}' is not a u64"))
                } else {
                    Err(format!(
                        "unknown checkpoint kind '{value}' (accepted: \
                         process-exit, first-rsx-write, pc=0xADDR)"
                    ))
                }
            }
        }
    }

    /// Read `--checkpoint <kind>` from a raw args vector. `None`
    /// means the flag was not supplied (caller uses the title
    /// default); `Some(Err)` means it was supplied but malformed.
    pub fn parse_from_args(args: &[String]) -> Option<Result<Self, String>> {
        for i in 0..args.len() {
            if args[i] == "--checkpoint" {
                let value = args.get(i + 1)?.as_str();
                return Some(Self::parse_cli_value(value));
            }
        }
        None
    }
}

/// On-disk TOML schema. Kept separate from [`TitleManifest`] so
/// the in-memory shape can evolve without breaking the file
/// format and vice versa. The loader translates one into the
/// other, validating checkpoint kinds at the boundary.
#[derive(Debug, serde::Deserialize)]
struct ManifestFile {
    title: ManifestTitle,
    checkpoint: ManifestCheckpoint,
}

#[derive(Debug, serde::Deserialize)]
struct ManifestTitle {
    content_id: String,
    short_name: String,
    display_name: String,
    eboot_candidates: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ManifestCheckpoint {
    kind: String,
    #[serde(default)]
    pc: Option<String>,
}

/// Why a manifest file failed to load. Each variant names the
/// file that went wrong and what the loader objected to; the
/// CLI surfaces these directly in error diagnostics.
#[derive(Debug)]
pub enum ManifestError {
    /// Reading the file from disk failed.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file is not valid TOML or does not match the schema.
    Parse { path: PathBuf, message: String },
    /// `checkpoint.kind` is not one of the known values.
    UnknownCheckpointKind { path: PathBuf, kind: String },
    /// `kind = "pc"` without a `pc = ...` value, or `pc` with a
    /// malformed value.
    BadCheckpointPc { path: PathBuf, detail: String },
    /// Two manifests share the same short name.
    DuplicateShortName {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    /// Two manifests share the same content id.
    DuplicateContentId {
        content_id: String,
        first: PathBuf,
        second: PathBuf,
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
            } => write!(
                f,
                "duplicate title short_name '{name}' in {} and {}",
                first.display(),
                second.display()
            ),
            Self::DuplicateContentId {
                content_id,
                first,
                second,
            } => write!(
                f,
                "duplicate title content_id '{content_id}' in {} and {}",
                first.display(),
                second.display()
            ),
        }
    }
}

impl TitleManifest {
    /// Load and validate a single manifest from a TOML file.
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::load_from_text(&text, path)
    }

    /// Parse a manifest from its in-memory TOML text, attaching
    /// `origin` to any error for diagnostics.
    pub fn load_from_text(text: &str, origin: &Path) -> Result<Self, ManifestError> {
        let file: ManifestFile = toml::from_str(text).map_err(|e| ManifestError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })?;
        let checkpoint =
            match file.checkpoint.kind.as_str() {
                "process-exit" => CheckpointTrigger::ProcessExit,
                "first-rsx-write" => CheckpointTrigger::FirstRsxWrite,
                "pc" => {
                    let raw = file.checkpoint.pc.as_ref().ok_or_else(|| {
                        ManifestError::BadCheckpointPc {
                            path: origin.to_path_buf(),
                            detail: "checkpoint kind 'pc' requires a 'pc = \"0xADDR\"' value"
                                .to_string(),
                        }
                    })?;
                    let hex = raw.trim_start_matches("0x").trim_start_matches("0X");
                    let parsed = u64::from_str_radix(hex, 16).or_else(|_| raw.parse::<u64>());
                    CheckpointTrigger::Pc(parsed.map_err(|_| ManifestError::BadCheckpointPc {
                        path: origin.to_path_buf(),
                        detail: format!("checkpoint pc value '{raw}' is not a u64"),
                    })?)
                }
                other => {
                    return Err(ManifestError::UnknownCheckpointKind {
                        path: origin.to_path_buf(),
                        kind: other.to_string(),
                    })
                }
            };
        Ok(TitleManifest {
            content_id: file.title.content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            checkpoint,
        })
    }

    /// Short CLI name for this title.
    pub fn name(&self) -> &str {
        &self.short_name
    }

    /// Human-readable label for diagnostics and log lines.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// PSN content id (directory name under `/dev_hdd0/game/`).
    pub fn content_id(&self) -> &str {
        &self.content_id
    }

    /// Candidate executable filenames inside `USRDIR/`, in
    /// priority order.
    pub fn eboot_candidates(&self) -> &[String] {
        &self.eboot_candidates
    }

    /// Built-in boot checkpoint default.
    pub fn checkpoint_trigger(&self) -> CheckpointTrigger {
        self.checkpoint
    }

    /// Build the conventional PS3 `USRDIR` path for this title
    /// under a VFS root (typically `/dev_hdd0`) and return the
    /// first candidate executable that exists on disk, or
    /// `None` if neither is present.
    pub fn resolve_eboot(&self, vfs_root: &Path) -> Option<PathBuf> {
        let usrdir = vfs_root.join("game").join(&self.content_id).join("USRDIR");
        for name in &self.eboot_candidates {
            let p = usrdir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// Registry of known titles, built by scanning a directory of
/// TOML manifests. Lookup by short name or by content id; both
/// are unique keys the loader validates at build time.
#[derive(Debug, Clone, Default)]
pub struct TitleRegistry {
    manifests: Vec<TitleManifest>,
}

// `iter`, `single_from_path`, and `is_empty` are part of the
// public surface used by tests and future Phase-13 tooling even
// though the current CLI only hits `by_short_name` /
// `by_content_id`.
#[allow(dead_code)]
impl TitleRegistry {
    /// Scan `dir` for `*.toml` files, load each as a
    /// [`TitleManifest`], and validate that short names and
    /// content ids are each globally unique. A missing or empty
    /// directory produces an empty registry; callers decide
    /// whether that is an error.
    ///
    /// Iteration order of the scan is sorted by filename so the
    /// registry's enumeration surface is deterministic across
    /// runs on the same disk. (Relevant for help text and error
    /// diagnostics, which list titles in registry order.)
    pub fn scan_dir(dir: &Path) -> Result<Self, ManifestError> {
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
                .collect(),
            Err(_) => return Ok(Self::default()),
        };
        entries.sort();

        let mut manifests: Vec<TitleManifest> = Vec::new();
        let mut short_names: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();
        let mut content_ids: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();
        for path in entries {
            let m = TitleManifest::load_from_path(&path)?;
            if let Some(prev) = short_names.get(&m.short_name) {
                return Err(ManifestError::DuplicateShortName {
                    name: m.short_name.clone(),
                    first: prev.clone(),
                    second: path.clone(),
                });
            }
            if let Some(prev) = content_ids.get(&m.content_id) {
                return Err(ManifestError::DuplicateContentId {
                    content_id: m.content_id.clone(),
                    first: prev.clone(),
                    second: path.clone(),
                });
            }
            short_names.insert(m.short_name.clone(), path.clone());
            content_ids.insert(m.content_id.clone(), path.clone());
            manifests.push(m);
        }
        Ok(Self { manifests })
    }

    /// Single-manifest registry built from one TOML file, used
    /// by the `--title-manifest <path>` flow so callers can
    /// point the harness at a manifest outside `docs/titles/`.
    pub fn single_from_path(path: &Path) -> Result<Self, ManifestError> {
        let m = TitleManifest::load_from_path(path)?;
        Ok(Self { manifests: vec![m] })
    }

    /// Whether the registry holds any manifests.
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Every manifest in the registry, in scan order.
    pub fn iter(&self) -> impl Iterator<Item = &TitleManifest> {
        self.manifests.iter()
    }

    /// Look up a manifest by its short name (e.g. `"sshd"`).
    /// Case-sensitive; returns `None` for unknown names.
    pub fn by_short_name(&self, name: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.short_name == name)
    }

    /// Look up a manifest by its PSN content id (e.g.
    /// `"NPUA80068"`). Returns `None` for unknown ids.
    pub fn by_content_id(&self, content_id: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.content_id == content_id)
    }

    /// Comma-separated list of every known title's short name in
    /// registry order. Used in CLI error diagnostics.
    pub fn known_names_csv(&self) -> String {
        self.manifests
            .iter()
            .map(|m| m.short_name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW_TOML: &str = r#"
[title]
content_id = "NPUA80001"
short_name = "flow"
display_name = "flOw (thatgamecompany, 2007)"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "process-exit"
"#;

    const SSHD_TOML: &str = r#"
[title]
content_id = "NPUA80068"
short_name = "sshd"
display_name = "Super Stardust HD (Housemarque, 2007)"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "first-rsx-write"
"#;

    const PC_TOML: &str = r#"
[title]
content_id = "NPEA00999"
short_name = "pcstop"
display_name = "PC-checkpoint test title"
eboot_candidates = ["EBOOT.elf"]

[checkpoint]
kind = "pc"
pc = "0x10381ce8"
"#;

    fn parse(text: &str) -> TitleManifest {
        TitleManifest::load_from_text(text, Path::new("test.toml")).unwrap()
    }

    #[test]
    fn parses_process_exit_manifest() {
        let m = parse(FLOW_TOML);
        assert_eq!(m.content_id, "NPUA80001");
        assert_eq!(m.short_name, "flow");
        assert_eq!(m.eboot_candidates, vec!["EBOOT.elf", "EBOOT.BIN"]);
        assert_eq!(m.checkpoint, CheckpointTrigger::ProcessExit);
    }

    #[test]
    fn parses_first_rsx_write_manifest() {
        let m = parse(SSHD_TOML);
        assert_eq!(m.content_id, "NPUA80068");
        assert_eq!(m.short_name, "sshd");
        assert_eq!(m.checkpoint, CheckpointTrigger::FirstRsxWrite);
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
    fn registry_scans_directory_in_sorted_order() {
        let tmp = std::env::temp_dir().join("cellgov_manifest_scan");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("NPUA80068.toml"), SSHD_TOML).unwrap();
        std::fs::write(tmp.join("NPUA80001.toml"), FLOW_TOML).unwrap();
        let reg = TitleRegistry::scan_dir(&tmp).unwrap();
        let names: Vec<&str> = reg.iter().map(|m| m.short_name.as_str()).collect();
        // Sorted by filename -> NPUA80001 comes before NPUA80068.
        assert_eq!(names, vec!["flow", "sshd"]);
        assert!(reg.by_short_name("flow").is_some());
        assert!(reg.by_content_id("NPUA80068").is_some());
        assert!(reg.by_short_name("unknown").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn registry_rejects_duplicate_short_names() {
        let tmp = std::env::temp_dir().join("cellgov_manifest_dupe_name");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // Two manifests with distinct content ids but the same
        // short_name -- registry build must fail.
        std::fs::write(tmp.join("a.toml"), FLOW_TOML).unwrap();
        let collide = FLOW_TOML.replace("NPUA80001", "NPEA12345");
        std::fs::write(tmp.join("b.toml"), &collide).unwrap();
        let err = TitleRegistry::scan_dir(&tmp).expect_err("duplicate short name");
        assert!(matches!(err, ManifestError::DuplicateShortName { .. }));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn registry_rejects_duplicate_content_ids() {
        let tmp = std::env::temp_dir().join("cellgov_manifest_dupe_cid");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.toml"), FLOW_TOML).unwrap();
        let collide = FLOW_TOML.replace(r#""flow""#, r#""flow2""#);
        std::fs::write(tmp.join("b.toml"), &collide).unwrap();
        let err = TitleRegistry::scan_dir(&tmp).expect_err("duplicate content id");
        assert!(matches!(err, ManifestError::DuplicateContentId { .. }));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn registry_scan_of_missing_dir_is_empty() {
        let p = Path::new("/nonexistent/cellgov/test/path/does/not/exist");
        let reg = TitleRegistry::scan_dir(p).unwrap();
        assert!(reg.is_empty());
    }

    #[test]
    fn checkpoint_parse_cli_forms() {
        assert_eq!(
            CheckpointTrigger::parse_cli_value("process-exit"),
            Ok(CheckpointTrigger::ProcessExit)
        );
        assert_eq!(
            CheckpointTrigger::parse_cli_value("first-rsx-write"),
            Ok(CheckpointTrigger::FirstRsxWrite)
        );
        assert_eq!(
            CheckpointTrigger::parse_cli_value("pc=0x10381ce8"),
            Ok(CheckpointTrigger::Pc(0x10381ce8))
        );
        assert!(CheckpointTrigger::parse_cli_value("nope").is_err());
        assert!(CheckpointTrigger::parse_cli_value("pc=xyz").is_err());
    }
}
