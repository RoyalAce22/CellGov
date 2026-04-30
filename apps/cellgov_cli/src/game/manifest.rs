//! Title registry driven by TOML manifests under `docs/titles/`.
//!
//! One TOML file per title (named for its PSN content id or disc
//! serial) carries content id, short name, display name, executable
//! candidate list, and boot checkpoint. Title metadata lives only
//! in `cellgov_cli`; no library crate below sees it.

use std::path::{Path, PathBuf};

/// How the title's executable is located on disk. Defaults to `Hdd`
/// when `[source]` is omitted; the loader does not infer disc from
/// the content id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameSource {
    /// EBOOT at `<vfs>/game/<content-id>/USRDIR/`.
    Hdd,
    /// EBOOT at `<vfs-parent>/dev_bdvd/<content-id>/PS3_GAME/USRDIR/`.
    /// Requires `vfs_root` to have a non-empty parent.
    Disc,
}

/// One title's manifest as loaded from `docs/titles/<content-id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleManifest {
    /// PSN content id; primary lookup key and the directory name
    /// under `/dev_hdd0/game/`.
    pub content_id: String,
    /// Short CLI name for `--title <name>`. Unique across the
    /// registry.
    pub short_name: String,
    pub display_name: String,
    /// Executable filenames tried in priority order under the
    /// title's `USRDIR/`. Decrypted ELFs listed before SELFs.
    pub eboot_candidates: Vec<String>,
    /// Built-in boot checkpoint; CLI `--checkpoint` overrides.
    pub checkpoint: CheckpointTrigger,
    pub source: GameSource,
    /// Maps the RSX region `ReadWrite` and enables
    /// [`cellgov_core::Runtime::set_rsx_mirror_writes`] so FIFO
    /// stores land in the RSX cursor for downstream advance.
    ///
    /// Mutually exclusive with `CheckpointTrigger::FirstRsxWrite`
    /// (a writable region cannot fault on the put-pointer store);
    /// the loader rejects both together.
    pub rsx_mirror: bool,
    /// Optional in-memory content provider. Read-only blobs the
    /// title expects to load from `/app_home/...` paths get
    /// registered in `Lv2Host::fs_store` at boot. Absent for
    /// titles that boot without title-specific content.
    pub content: Option<ContentManifest>,
}

/// Per-title content provider entries, each mapping a guest path
/// the title opens via `sys_fs_open` to a host file whose bytes
/// get registered in `Lv2Host::fs_store` at boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentManifest {
    /// Base directory for relative `host_path`s. Resolved against
    /// the workspace root when relative; used as-is when absolute.
    pub base: String,
    /// Optional environment variable name. When set in the
    /// process environment to a non-empty value, that value is
    /// used as the base directory instead of [`Self::base`]. The
    /// override exists for the gitignored-developer-local content
    /// path: a developer who has the real title content drops it
    /// into a local directory and exports the env var, without
    /// needing to edit the committed manifest.
    pub override_base_env: Option<String>,
    pub files: Vec<ContentEntry>,
}

/// One blob to register: `guest_path` is what the title sees from
/// `sys_fs_open`; `host_path` is the on-disk source (resolved
/// against the [`ContentManifest::base`] when relative).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentEntry {
    pub guest_path: String,
    pub host_path: String,
}

/// Stop condition for a boot. Default comes from the manifest;
/// `--checkpoint` overrides per run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointTrigger {
    /// Stop on `sys_process_exit`.
    ProcessExit,
    /// Stop on the first PPU write into the RSX region. The
    /// resulting `MemError::ReservedWrite { region: "rsx", .. }`
    /// is classified as "checkpoint reached", not a fault.
    FirstRsxWrite,
    /// Stop when a step retires at the given guest PC. Only
    /// selectable via `--checkpoint pc=0xADDR`.
    Pc(u64),
}

impl CheckpointTrigger {
    /// Parse a `--checkpoint <kind>` value. Accepts `process-exit`,
    /// `first-rsx-write`, `pc=0xHEX`, or `pc=DECIMAL`. Hex requires
    /// the `0x`/`0X` prefix so `pc=10` is unambiguously decimal.
    pub fn parse_cli_value(value: &str) -> Result<Self, String> {
        match value {
            "process-exit" => Ok(Self::ProcessExit),
            "first-rsx-write" => Ok(Self::FirstRsxWrite),
            _ => {
                if let Some(rest) = value.strip_prefix("pc=") {
                    parse_pc_literal(rest).map(Self::Pc)
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
    /// means the flag was absent; `Some(Err)` means it was
    /// supplied malformed, repeated, or missing its value.
    pub fn parse_from_args(args: &[String]) -> Option<Result<Self, String>> {
        let mut found: Option<Result<Self, String>> = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] != "--checkpoint" {
                i += 1;
                continue;
            }
            if found.is_some() {
                return Some(Err(
                    "--checkpoint was specified more than once; pass it exactly once.".to_string(),
                ));
            }
            let parsed = match args.get(i + 1) {
                Some(v) => Self::parse_cli_value(v.as_str()),
                None => Err(
                    "--checkpoint requires a value (process-exit, first-rsx-write, \
                     or pc=0xADDR)"
                        .to_string(),
                ),
            };
            found = Some(parsed);
            // Skip the value token: a second `--checkpoint` used as
            // the value must not be rematched as the flag.
            i += 2;
        }
        found
    }
}

/// Parse a PC literal from `--checkpoint pc=...` or a manifest
/// `pc = "..."`. Hex requires `0x`/`0X`; otherwise decimal. Same
/// rule on both sides so CLI and manifest cannot disagree.
fn parse_pc_literal(raw: &str) -> Result<u64, String> {
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
            .map_err(|_| format!("checkpoint pc value '{raw}' is not a hex u64"))
    } else {
        raw.parse::<u64>().map_err(|_| {
            format!("checkpoint pc value '{raw}' is not a decimal u64 (use 0x prefix for hex)")
        })
    }
}

/// On-disk TOML schema, translated to [`TitleManifest`] at load.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    title: ManifestTitle,
    checkpoint: ManifestCheckpoint,
    source: Option<ManifestSource>,
    rsx: Option<ManifestRsx>,
    content: Option<ManifestContent>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestContent {
    base: String,
    #[serde(default)]
    override_base_env: Option<String>,
    files: Vec<ManifestContentFile>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestContentFile {
    guest_path: String,
    host_path: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestRsx {
    #[serde(default)]
    mirror: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestSource {
    kind: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestTitle {
    content_id: String,
    short_name: String,
    display_name: String,
    eboot_candidates: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCheckpoint {
    kind: String,
    #[serde(default)]
    pc: Option<String>,
}

/// Why [`TitleManifest::resolve_eboot`] could not return a path.
/// Distinct variants let callers separate "not installed" from
/// "vfs-root is misconfigured" from I/O errors during probing.
#[derive(Debug)]
pub enum ResolveEbootError {
    /// Disc title with a `vfs_root` that has no non-empty parent,
    /// so `dev_bdvd/<content-id>/...` cannot be located.
    MisconfiguredVfsRoot {
        vfs_root: PathBuf,
        short_name: String,
    },
    /// No candidate executable exists under the resolved USRDIR.
    /// `probe_errors` collects non-NotFound I/O errors (permission
    /// denied, broken symlink, stale NFS handle); usually empty.
    NotFound {
        searched: PathBuf,
        candidates: Vec<String>,
        probe_errors: Vec<(PathBuf, std::io::Error)>,
    },
}

impl std::fmt::Display for ResolveEbootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MisconfiguredVfsRoot {
                vfs_root,
                short_name,
            } => write!(
                f,
                "disc title '{short_name}' needs vfs-root with a parent directory (got {})",
                vfs_root.display()
            ),
            Self::NotFound {
                searched,
                candidates,
                probe_errors,
            } => {
                write!(f, "no executable found; looked for:")?;
                for name in candidates {
                    write!(f, "\n  {}", searched.join(name).display())?;
                }
                for (p, e) in probe_errors {
                    write!(f, "\n  probe error: {}: {e}", p.display())?;
                }
                Ok(())
            }
        }
    }
}

/// Why a manifest file failed to load.
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
        /// Files are byte-identical on disk; hint that one is
        /// likely a stray copy rather than a real conflict.
        files_identical: bool,
    },
    DuplicateContentId {
        content_id: String,
        first: PathBuf,
        second: PathBuf,
        files_identical: bool,
    },
}

/// True iff both reads succeed and contents match; silent false on
/// I/O error since this is a diagnostic hint. Short-circuits on
/// `metadata` length before slurping both files into memory.
fn files_have_identical_bytes(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {}
        _ => return false,
    }
    matches!(
        (std::fs::read(a), std::fs::read(b)),
        (Ok(x), Ok(y)) if x == y
    )
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

impl TitleManifest {
    /// Read and parse a manifest from a TOML file.
    pub fn load_from_path(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::load_from_text(&text, path)
    }

    /// Parse a manifest from in-memory TOML text, attaching
    /// `origin` to any error.
    ///
    /// Accepts two layouts: root-level tables (`[title]`,
    /// `[checkpoint]`, optional `[source]`), or the same tables
    /// nested under `[cellgov]` so microtests can co-locate the
    /// CellGov manifest with RPCS3 harness config in one file.
    pub fn load_from_text(text: &str, origin: &Path) -> Result<Self, ManifestError> {
        let raw: toml::Value = toml::from_str(text).map_err(|e| ManifestError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })?;
        // A `cellgov` key selects the nested layout; scalar/array
        // forms are shape errors, and root-level title tables
        // alongside `[cellgov.*]` are ambiguous.
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
                let conflicting: Vec<&str> = ["title", "checkpoint", "source", "rsx", "content"]
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
                    let parsed =
                        parse_pc_literal(raw).map_err(|detail| ManifestError::BadCheckpointPc {
                            path: origin.to_path_buf(),
                            detail,
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
            // "hdd" as an explicit synonym for the default.
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
        Ok(TitleManifest {
            content_id: file.title.content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            checkpoint,
            source,
            rsx_mirror,
            content,
        })
    }

    pub fn name(&self) -> &str {
        &self.short_name
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn checkpoint_trigger(&self) -> CheckpointTrigger {
        self.checkpoint
    }

    /// See the [`TitleManifest::rsx_mirror`] field doc.
    pub fn rsx_mirror(&self) -> bool {
        self.rsx_mirror
    }

    /// Return the first [`eboot_candidates`] filename that exists
    /// as a regular file under the title's USRDIR.
    ///
    /// [`eboot_candidates`]: TitleManifest::eboot_candidates
    ///
    /// # Errors
    ///
    /// See [`ResolveEbootError`] for the three failure modes. The
    /// function never prints; the caller renders.
    pub fn resolve_eboot(&self, vfs_root: &Path) -> Result<PathBuf, ResolveEbootError> {
        let usrdir = match self.source {
            GameSource::Hdd => vfs_root.join("game").join(&self.content_id).join("USRDIR"),
            GameSource::Disc => {
                let parent = match vfs_root.parent() {
                    Some(p) if !p.as_os_str().is_empty() => p,
                    _ => {
                        return Err(ResolveEbootError::MisconfiguredVfsRoot {
                            vfs_root: vfs_root.to_path_buf(),
                            short_name: self.short_name.clone(),
                        });
                    }
                };
                parent
                    .join("dev_bdvd")
                    .join(&self.content_id)
                    .join("PS3_GAME")
                    .join("USRDIR")
            }
        };
        let mut probe_errors = Vec::new();
        for name in &self.eboot_candidates {
            let p = usrdir.join(name);
            match std::fs::metadata(&p) {
                Ok(md) if md.is_file() => return Ok(p),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => probe_errors.push((p, e)),
            }
        }
        Err(ResolveEbootError::NotFound {
            searched: usrdir,
            candidates: self.eboot_candidates.clone(),
            probe_errors,
        })
    }
}

/// Registry of known titles, built by scanning a directory of
/// TOML manifests. Short name and content id are both unique keys.
#[derive(Debug, Clone, Default)]
pub struct TitleRegistry {
    manifests: Vec<TitleManifest>,
}

impl TitleRegistry {
    /// Load every `*.toml` under `dir` as a [`TitleManifest`] and
    /// validate short-name and content-id uniqueness. A missing
    /// `dir` yields an empty registry; other I/O errors surface.
    /// Entries are sorted by filename so iteration order is
    /// deterministic.
    pub fn scan_dir(dir: &Path) -> Result<Self, ManifestError> {
        // Only NotFound collapses to an empty registry; other
        // errors (permission-denied, not-a-directory) must surface.
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(ManifestError::Io {
                    path: dir.to_path_buf(),
                    source,
                })
            }
        };

        let mut entries: Vec<PathBuf> = Vec::new();
        for entry in rd {
            let entry = entry.map_err(|source| ManifestError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            // Case-insensitive extension; skip dotfiles so
            // backup/tooling files like `.backup.toml` do not load.
            let is_toml = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
            let is_hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if is_toml && !is_hidden {
                entries.push(path);
            }
        }
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
                    files_identical: files_have_identical_bytes(prev, &path),
                });
            }
            if let Some(prev) = content_ids.get(&m.content_id) {
                return Err(ManifestError::DuplicateContentId {
                    content_id: m.content_id.clone(),
                    first: prev.clone(),
                    second: path.clone(),
                    files_identical: files_have_identical_bytes(prev, &path),
                });
            }
            short_names.insert(m.short_name.clone(), path.clone());
            content_ids.insert(m.content_id.clone(), path.clone());
            manifests.push(m);
        }
        Ok(Self { manifests })
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &TitleManifest> {
        self.manifests.iter()
    }

    /// Look up a manifest by case-sensitive short name.
    pub fn by_short_name(&self, name: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.short_name == name)
    }

    /// Look up a manifest by PSN content id or disc serial.
    pub fn by_content_id(&self, content_id: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.content_id == content_id)
    }

    /// Comma-separated short names in registry order, or
    /// `"<none>"` when empty so CLI error messages stay legible.
    pub fn known_names_csv(&self) -> String {
        if self.manifests.is_empty() {
            return "<none>".to_string();
        }
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

    /// RAII tempdir suffixed with the process id so concurrent
    /// `cargo test` runs do not collide.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("cellgov_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const PROCESS_EXIT_TOML: &str = r#"
[title]
content_id = "NPAA00001"
short_name = "proc-exit-fixture"
display_name = "Process-exit checkpoint fixture"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "process-exit"
"#;

    const FIRST_RSX_WRITE_TOML: &str = r#"
[title]
content_id = "NPAA00002"
short_name = "rsx-write-fixture"
display_name = "First-RSX-write checkpoint fixture"
eboot_candidates = ["EBOOT.elf", "EBOOT.BIN"]

[checkpoint]
kind = "first-rsx-write"
"#;

    const PC_TOML: &str = r#"
[title]
content_id = "NPAA00003"
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
        let m = parse(PROCESS_EXIT_TOML);
        assert_eq!(m.content_id, "NPAA00001");
        assert_eq!(m.short_name, "proc-exit-fixture");
        assert_eq!(m.eboot_candidates, vec!["EBOOT.elf", "EBOOT.BIN"]);
        assert_eq!(m.checkpoint, CheckpointTrigger::ProcessExit);
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
        // An empty `files` array is a valid (if pointless) shape;
        // it lets a manifest scaffold the [content] block before
        // any blobs are wired.
        let text = r#"
[title]
content_id = "NPAA77778"
short_name = "empty-content"
display_name = "Empty content"
eboot_candidates = ["EBOOT.elf"]

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
        // serde(deny_unknown_fields) catches typos like
        // `host-path` (dash instead of underscore).
        let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]

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
        let tmp = TmpDir::new("manifest_scan");
        std::fs::write(tmp.path().join("NPAA00002.toml"), FIRST_RSX_WRITE_TOML).unwrap();
        std::fs::write(tmp.path().join("NPAA00001.toml"), PROCESS_EXIT_TOML).unwrap();
        let reg = TitleRegistry::scan_dir(tmp.path()).unwrap();
        let names: Vec<&str> = reg.iter().map(|m| m.short_name.as_str()).collect();
        assert_eq!(names, vec!["proc-exit-fixture", "rsx-write-fixture"]);
        assert!(reg.by_short_name("proc-exit-fixture").is_some());
        assert!(reg.by_content_id("NPAA00002").is_some());
        assert!(reg.by_short_name("unknown").is_none());
    }

    #[test]
    fn registry_rejects_duplicate_short_names() {
        let tmp = TmpDir::new("manifest_dupe_name");
        std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
        let collide = PROCESS_EXIT_TOML.replace("NPAA00001", "NPAA99999");
        std::fs::write(tmp.path().join("b.toml"), &collide).unwrap();
        let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate short name");
        assert!(matches!(err, ManifestError::DuplicateShortName { .. }));
    }

    #[test]
    fn registry_rejects_duplicate_content_ids() {
        let tmp = TmpDir::new("manifest_dupe_cid");
        std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
        let collide =
            PROCESS_EXIT_TOML.replace(r#""proc-exit-fixture""#, r#""proc-exit-fixture-2""#);
        std::fs::write(tmp.path().join("b.toml"), &collide).unwrap();
        let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate content id");
        assert!(matches!(err, ManifestError::DuplicateContentId { .. }));
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

    #[test]
    fn checkpoint_unprefixed_digits_parse_as_decimal_not_hex() {
        assert_eq!(
            CheckpointTrigger::parse_cli_value("pc=10"),
            Ok(CheckpointTrigger::Pc(10))
        );
    }

    #[test]
    fn checkpoint_unprefixed_hex_is_rejected() {
        assert!(CheckpointTrigger::parse_cli_value("pc=1ce8").is_err());
    }

    #[test]
    fn parse_from_args_rejects_repeated_flag() {
        let args = vec![
            "run-game".to_string(),
            "--checkpoint".to_string(),
            "process-exit".to_string(),
            "--checkpoint".to_string(),
            "first-rsx-write".to_string(),
        ];
        let got = CheckpointTrigger::parse_from_args(&args);
        assert!(
            matches!(got, Some(Err(_))),
            "repeated --checkpoint must surface as Some(Err)"
        );
    }

    #[test]
    fn parse_from_args_rejects_missing_value() {
        let args = vec!["run-game".to_string(), "--checkpoint".to_string()];
        let got = CheckpointTrigger::parse_from_args(&args);
        assert!(
            matches!(got, Some(Err(_))),
            "--checkpoint with no value must be Some(Err), not None"
        );
    }

    #[test]
    fn parse_from_args_returns_none_when_flag_absent() {
        let args = vec!["run-game".to_string(), "--other".to_string()];
        assert!(CheckpointTrigger::parse_from_args(&args).is_none());
    }

    #[test]
    fn pc_manifest_accepts_decimal_literal() {
        let text = r#"
[title]
content_id = "x"
short_name = "x"
display_name = "x"
eboot_candidates = ["x"]

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

[checkpoint]
kind = "process-exit"

[cellgov.title]
content_id = "nested"
short_name = "nested"
display_name = "nested"
eboot_candidates = ["y"]

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

    #[test]
    fn known_names_csv_empty_registry_is_labelled() {
        let reg = TitleRegistry::default();
        assert_eq!(reg.known_names_csv(), "<none>");
    }

    fn hdd_manifest(content_id: &str, short: &str, candidates: &[&str]) -> TitleManifest {
        TitleManifest {
            content_id: content_id.to_string(),
            short_name: short.to_string(),
            display_name: short.to_string(),
            eboot_candidates: candidates.iter().map(|s| s.to_string()).collect(),
            checkpoint: CheckpointTrigger::ProcessExit,
            source: GameSource::Hdd,
            rsx_mirror: false,
            content: None,
        }
    }

    #[test]
    fn resolve_eboot_hdd_finds_first_candidate() {
        let tmp = TmpDir::new("resolve_hdd_first");
        let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        std::fs::write(usrdir.join("EBOOT.elf"), b"elf").unwrap();
        std::fs::write(usrdir.join("EBOOT.BIN"), b"bin").unwrap();
        let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
        let got = m
            .resolve_eboot(tmp.path())
            .expect("first candidate resolves");
        assert_eq!(got, usrdir.join("EBOOT.elf"));
    }

    #[test]
    fn resolve_eboot_hdd_falls_through_to_second_candidate() {
        let tmp = TmpDir::new("resolve_hdd_fallthrough");
        let usrdir = tmp.path().join("game").join("NPAA00001").join("USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        std::fs::write(usrdir.join("EBOOT.BIN"), b"bin").unwrap();
        let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
        let got = m
            .resolve_eboot(tmp.path())
            .expect("second candidate resolves");
        assert_eq!(got, usrdir.join("EBOOT.BIN"));
    }

    #[test]
    fn resolve_eboot_hdd_returns_notfound_with_candidate_list() {
        let tmp = TmpDir::new("resolve_hdd_notfound");
        let m = hdd_manifest("NPAA00001", "t", &["EBOOT.elf", "EBOOT.BIN"]);
        match m.resolve_eboot(tmp.path()) {
            Err(ResolveEbootError::NotFound {
                candidates,
                probe_errors,
                ..
            }) => {
                assert_eq!(candidates, vec!["EBOOT.elf", "EBOOT.BIN"]);
                assert!(probe_errors.is_empty(), "no probe errors expected");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn resolve_eboot_disc_without_parent_returns_misconfigured() {
        // "dev_hdd0" has `parent() == Some("")`; "/" and "" return
        // `parent() == None`. Both must yield MisconfiguredVfsRoot.
        let mut m = hdd_manifest("NPAA00001", "disc-t", &["EBOOT.BIN"]);
        m.source = GameSource::Disc;
        for bad in ["dev_hdd0", "/", ""] {
            let err = m.resolve_eboot(Path::new(bad)).expect_err("needs parent");
            assert!(
                matches!(err, ResolveEbootError::MisconfiguredVfsRoot { .. }),
                "vfs_root={bad:?} must yield MisconfiguredVfsRoot, got {err:?}"
            );
        }
    }

    #[test]
    fn duplicate_detection_flags_byte_identical_files() {
        let tmp = TmpDir::new("manifest_identical_dupes");
        std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
        std::fs::write(tmp.path().join("b.toml"), PROCESS_EXIT_TOML).unwrap();
        let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate");
        match err {
            ManifestError::DuplicateShortName {
                files_identical, ..
            }
            | ManifestError::DuplicateContentId {
                files_identical, ..
            } => assert!(files_identical, "identical files must set the hint"),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
