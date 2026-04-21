//! Title registry driven by TOML manifests under `docs/titles/`.
//!
//! Every PS3 title the CLI's game harness knows about corresponds
//! to one TOML file named after its PSN content id or disc serial
//! (e.g. `docs/titles/<SERIAL>.toml`). The file carries the four
//! facts the harness cares about: content id, short name, display
//! name, and the main-executable candidate list; plus the default
//! boot checkpoint. Adding a new title that fits the existing
//! checkpoint kinds and the standard PS3 VFS layout is a
//! single-file commit -- no Rust change.
//!
//! Title metadata lives only in `cellgov_cli`; no library crate
//! below knows titles exist. Downstream importers that pull
//! `cellgov_ppu` or `cellgov_compare` do not transitively pull a
//! registry of named games.

use std::path::{Path, PathBuf};

/// How the title's executable is located on disk.
///
/// Omitting the `[source]` table in a manifest defaults to `Hdd`.
/// Disc titles must spell out `kind = "disc"` in `[source]`; the
/// loader does not infer disc from the content id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameSource {
    /// PSN / HDD game: EBOOT in `<vfs>/game/<content-id>/USRDIR/`.
    /// This is the default when `[source]` is omitted.
    Hdd,
    /// Disc game: EBOOT in `<vfs-parent>/dev_bdvd/<content-id>/PS3_GAME/USRDIR/`.
    /// The vfs root passed to `resolve_eboot` must have a non-empty
    /// parent directory (typically `vfs_root` itself is
    /// `.../dev_hdd0` and the parent contains both `dev_hdd0` and
    /// `dev_bdvd`).
    Disc,
}

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
    /// Short CLI name for `--title <name>` and diagnostics. Must
    /// be unique across the registry; the loader rejects
    /// duplicates.
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
    /// Where to find the executable on disk.
    pub source: GameSource,
    /// When `true`, the title runs with the PS3 RSX region mapped
    /// `ReadWrite` (not the default `ReservedZeroReadable`) AND
    /// with [`cellgov_core::Runtime::set_rsx_mirror_writes`]
    /// enabled. The guest's put-pointer write at
    /// `0xC000_0040` succeeds and is mirrored into the runtime's
    /// RSX cursor; the FIFO advance pass then drains the
    /// command buffer.
    ///
    /// Mutually exclusive with the `FirstRsxWrite` checkpoint
    /// kind: that checkpoint expects the put-pointer write to
    /// fault, which cannot happen when the region is writable.
    /// The manifest loader rejects manifests that request both.
    /// Microtests that script FIFO writes set this `true`;
    /// default bench manifests leave it `false` so the
    /// FirstRsxWrite checkpoint still fires.
    pub rsx_mirror: bool,
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
    /// `process-exit`, `first-rsx-write`, and `pc=0xHEX` or
    /// `pc=DECIMAL`. Hex requires the explicit `0x` / `0X` prefix;
    /// unprefixed values parse as decimal. Without this rule
    /// `pc=10` is ambiguous (ten or sixteen?) and a user who drops
    /// the `0x` gets a silently different address.
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
    /// means the flag was not supplied (caller uses the title
    /// default); `Some(Err)` means it was supplied but malformed,
    /// repeated, or missing its value.
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
            // Skip the value token so it cannot accidentally match a
            // flag name if a user picks `--checkpoint --checkpoint`.
            i += 2;
        }
        found
    }
}

/// Parse a PC literal from a `--checkpoint pc=...` CLI value or a
/// `pc = "..."` manifest string. `0x`/`0X` prefix is required for
/// hex; unprefixed values parse as decimal. Same rule both sides so
/// the CLI override and the manifest default cannot silently disagree
/// on what `pc=1ce8` means.
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

/// On-disk TOML schema. Kept separate from [`TitleManifest`] so
/// the in-memory shape can evolve without breaking the file
/// format and vice versa. The loader translates one into the
/// other, validating checkpoint kinds at the boundary.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    title: ManifestTitle,
    checkpoint: ManifestCheckpoint,
    source: Option<ManifestSource>,
    rsx: Option<ManifestRsx>,
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
///
/// Kept distinct from the generic `Option<PathBuf>` of the old API
/// so callers (CLI, tests, anything that cares about stderr) can
/// distinguish "title is not installed" from "vfs-root is
/// misconfigured" from "probe hit an I/O error we should show."
/// The function itself never prints; rendering is the caller's
/// choice.
#[derive(Debug)]
pub enum ResolveEbootError {
    /// The manifest specifies a disc title but the supplied
    /// `vfs_root` has no non-empty parent directory, so
    /// `dev_bdvd/<content-id>/...` cannot be located.
    MisconfiguredVfsRoot {
        vfs_root: PathBuf,
        short_name: String,
    },
    /// None of the candidate executables exist under the resolved
    /// USRDIR. `probe_errors` collects I/O errors other than
    /// not-found encountered while scanning (permission denied,
    /// broken symlink, stale NFS handle); the common case is an
    /// empty Vec, meaning the files simply are not present.
    ///
    /// `probe_errors` is not exercised by unit tests -- reliably
    /// producing a non-NotFound io::Error from `metadata()` in a
    /// portable way is awkward (EACCES on the parent directory is
    /// the usual real-world trigger). In practice this field is
    /// populated by permission problems on USRDIR; treat it as the
    /// operator-facing diagnostic channel for those cases, not as
    /// dead code.
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
        /// When `true`, the two files are byte-identical on disk;
        /// the operator is probably looking at a stray backup or
        /// copy rather than two genuinely distinct manifests.
        files_identical: bool,
    },
    /// Two manifests share the same content id.
    DuplicateContentId {
        content_id: String,
        first: PathBuf,
        second: PathBuf,
        /// True when the two manifests are byte-identical
        /// duplicates of each other (benign -- copy/paste case);
        /// false when they differ and the conflict is a real
        /// configuration error.
        files_identical: bool,
    },
}

/// Compare two files byte-for-byte, returning `true` only if both
/// reads succeed and the contents match. Silent `false` on I/O
/// error is intentional: this flag is a diagnostic hint, not an
/// assertion, and a read failure here should not change how the
/// duplicate error surfaces elsewhere. Short-circuits on `metadata`
/// length so a stray large file that happens to have a `.toml`
/// extension is not slurped into memory twice just for a hint.
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
    ///
    /// Supports two layouts for the same shape:
    ///
    /// 1. Root-level (the standard docs/titles/ format): a
    ///    `[title]`, `[checkpoint]`, optional `[source]` table at
    ///    the top of the file.
    /// 2. Nested under `[cellgov]`: same tables packed under a
    ///    `[cellgov]` key. Used by microtests that co-locate the
    ///    CellGov title manifest with their RPCS3 test-harness
    ///    manifest in a single TOML file.
    pub fn load_from_text(text: &str, origin: &Path) -> Result<Self, ManifestError> {
        let raw: toml::Value = toml::from_str(text).map_err(|e| ManifestError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })?;
        // Layout selection: if a `cellgov` key exists it must be a
        // table (the nested microtest layout). A scalar or array
        // under that key is a shape error, not a silent trigger to
        // drop the root-level view. And when nested mode is chosen,
        // any root-level `title`/`checkpoint`/`source` tables are
        // flagged -- otherwise they would be silently discarded and
        // half the file's contents vanish without warning.
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
                let conflicting: Vec<&str> = ["title", "checkpoint", "source", "rsx"]
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
            // "hdd" is accepted as an explicit synonym for the
            // default so maintainers can spell it out for symmetry
            // with disc manifests without getting an "unknown
            // source kind" error.
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
        Ok(TitleManifest {
            content_id: file.title.content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            checkpoint,
            source,
            rsx_mirror,
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

    /// Built-in boot checkpoint default.
    pub fn checkpoint_trigger(&self) -> CheckpointTrigger {
        self.checkpoint
    }

    /// Whether this title runs with the RSX region writable and
    /// the runtime's writeback mirror enabled. See the
    /// [`TitleManifest::rsx_mirror`] field doc for the detailed
    /// semantics.
    pub fn rsx_mirror(&self) -> bool {
        self.rsx_mirror
    }

    /// Build the conventional PS3 `USRDIR` path for this title
    /// under a VFS root (typically `/dev_hdd0`) and return the
    /// first candidate executable that exists on disk.
    ///
    /// Returns a structured [`ResolveEbootError`] on failure
    /// rather than `None`; the caller decides how to render the
    /// three failure modes (misconfigured vfs root, no candidate
    /// exists, probe I/O errors). The function itself never
    /// prints, so tests and non-CLI consumers stay quiet.
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
                Ok(_) => {} // exists but not a regular file; keep scanning
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
/// TOML manifests. Lookup by short name or by content id; both
/// are unique keys the loader validates at build time.
#[derive(Debug, Clone, Default)]
pub struct TitleRegistry {
    manifests: Vec<TitleManifest>,
}

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
        // Only NotFound is "empty registry"; permission-denied or
        // not-a-directory are real errors and must not be laundered
        // into an empty result. A typo'd directory used to produce
        // the same "empty registry" as a legitimately missing one.
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
            // Case-insensitive ".toml" so a mixed-case filesystem
            // does not swallow `FOO.TOML`. Skip hidden files so
            // manually-hidden or tooling-created dotfile manifests
            // (e.g. `.backup.toml`) do not silently load as real
            // titles. Vim swap files like `.foo.toml.swp` already
            // fail the extension check; Emacs `#name.toml#`
            // lockfiles fail it too (trailing `#` makes extension
            // parse as `toml#`).
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

    /// Whether the registry holds any manifests. Used by tests
    /// and the `scan_dir_of_missing_dir_is_empty` check; kept
    /// distinct from `manifests.is_empty()` so the public API
    /// stays self-contained.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Every manifest in the registry, in scan order. Used by
    /// tests; no CLI caller enumerates the registry directly.
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &TitleManifest> {
        self.manifests.iter()
    }

    /// Look up a manifest by its short name (e.g. `"sshd"`).
    /// Case-sensitive; returns `None` for unknown names.
    pub fn by_short_name(&self, name: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.short_name == name)
    }

    /// Look up a manifest by its PSN content id or disc serial
    /// (e.g. an `NPxxNNNNN` PSN id or a `BCxxNNNNN` disc serial).
    /// Returns `None` for unknown ids.
    pub fn by_content_id(&self, content_id: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.content_id == content_id)
    }

    /// Comma-separated list of every known title's short name in
    /// registry order. Used in CLI error diagnostics. Returns
    /// `"<none>"` rather than an empty string when the registry
    /// is empty so CLI messages like
    /// `unknown title 'xyz' (known: <none>)` are legible instead
    /// of rendering as `(known: )` which looks like broken output.
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

    /// RAII tempdir helper. Each instance uses a unique directory
    /// suffixed with the current process id so concurrent `cargo
    /// test` invocations do not stomp on each other's fixtures.
    /// `remove_dir_all` on drop means a panicking test still
    /// leaves the filesystem clean on the next run.
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

    // Synthetic TOML fixtures exercising each checkpoint kind. The
    // content ids and short names are placeholder values -- the
    // registry treats every title the same, so the TOML layout and
    // checkpoint parsing are what matter.
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
        // Microtest convention: one TOML file carries both the
        // RPCS3-harness manifest (at root) and the CellGov title
        // manifest (under `[cellgov]`). The title loader pulls
        // the nested subtree transparently.
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
        // Mutually-exclusive pair: FirstRsxWrite expects the put
        // store to fault; rsx_mirror makes the region writable so
        // it cannot. Loader must reject.
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
        // Sorted by filename -> NPAA00001 comes before NPAA00002.
        assert_eq!(names, vec!["proc-exit-fixture", "rsx-write-fixture"]);
        assert!(reg.by_short_name("proc-exit-fixture").is_some());
        assert!(reg.by_content_id("NPAA00002").is_some());
        assert!(reg.by_short_name("unknown").is_none());
    }

    #[test]
    fn registry_rejects_duplicate_short_names() {
        let tmp = TmpDir::new("manifest_dupe_name");
        // Two manifests with distinct content ids but the same
        // short_name -- registry build must fail.
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
        // Must be decimal ten, not hex sixteen. Without the strict
        // prefix rule the two answers agreed with each other because
        // the hex fallback silently coerced decimal-looking values.
        assert_eq!(
            CheckpointTrigger::parse_cli_value("pc=10"),
            Ok(CheckpointTrigger::Pc(10))
        );
    }

    #[test]
    fn checkpoint_unprefixed_hex_is_rejected() {
        // Previously `pc=1ce8` silently parsed as 0x1ce8 via the
        // decimal-fallback-to-hex chain. Now the decimal parse
        // fails and no hex fallback is attempted.
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
        // Decimal 256, not hex 0x256.
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
        // Presence of a root-level [title] alongside [cellgov.*] is
        // ambiguous: the old loader silently dropped root-level
        // tables. Now both layouts present at once is a shape error.
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
        // Only the second candidate exists.
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
        // Two triggers converge on the same MisconfiguredVfsRoot
        // variant; the test locks in both. A single-component
        // relative path like "dev_hdd0" has `parent() == Some("")`
        // (empty but non-None), while "/" and "" return
        // `parent() == None`. If either trigger drifts under
        // refactor the test turns red.
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
        // Two literally identical files will collide on both
        // short_name and content_id; the error must carry the
        // byte-identical hint to save operators from diffing them.
        std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
        std::fs::write(tmp.path().join("b.toml"), PROCESS_EXIT_TOML).unwrap();
        let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate");
        match err {
            ManifestError::DuplicateShortName {
                files_identical, ..
            }
            | ManifestError::DuplicateContentId {
                files_identical, ..
            } => assert!(
                files_identical,
                "identical files must set the files_identical hint"
            ),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
