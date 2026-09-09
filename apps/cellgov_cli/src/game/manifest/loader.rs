//! TOML -> [`TitleManifest`] translation.

use std::path::{Path, PathBuf};

use super::checkpoint::{parse_pc_literal, CheckpointTrigger};
use super::matrix;
use super::model::{
    ContentEntry, ContentManifest, Distribution, GameSource, MountEntry, TitleManifest,
};
use super::schema::{ManifestCheckpoint, ManifestFile};

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse {}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error(
        "{}: unknown checkpoint kind '{kind}' (accepted: process-exit, first-rsx-write, pc)",
        path.display()
    )]
    UnknownCheckpointKind { path: PathBuf, kind: String },
    #[error("{}: {detail}", path.display())]
    BadCheckpointPc { path: PathBuf, detail: String },
    #[error(
        "duplicate title short_name '{name}' in {} and {}{}",
        first.display(),
        second.display(),
        render_files_identical_hint(*files_identical)
    )]
    DuplicateShortName {
        name: String,
        first: PathBuf,
        second: PathBuf,
        files_identical: bool,
    },
    #[error(
        "duplicate title content_id '{content_id}' in {} and {}{}",
        first.display(),
        second.display(),
        render_files_identical_hint(*files_identical)
    )]
    DuplicateContentId {
        content_id: String,
        first: PathBuf,
        second: PathBuf,
        files_identical: bool,
    },
}

/// Root-level table names the nested `[cellgov]` layout would shadow.
/// Must stay equal to [`ManifestFile`]'s field set: a table present in
/// the struct but missing here is read from `[cellgov]` and silently
/// dropped from the root. `root_table_keys_cover_every_manifest_table`
/// holds the two together.
const ROOT_TABLE_KEYS: [&str; 7] = [
    "title",
    "checkpoint",
    "source",
    "rsx",
    "content",
    "fs",
    "bench",
];

/// Directory holding `origin`, or `.` for a bare filename. Joining
/// onto `.` keeps a manifest-relative path relative instead of
/// silently rooting it at the process cwd.
fn manifest_dir(origin: &Path) -> PathBuf {
    match origin.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// True when `raw` carries a drive prefix or a leading separator, i.e.
/// when `Path::join` would drop the base it is being joined onto rather
/// than extend it. Covers the Windows drive-relative form (`C:build`)
/// as well as rooted (`\build`) and fully absolute paths, which
/// `Path::is_absolute` alone does not.
fn discards_its_base(raw: &str) -> bool {
    use std::path::Component;
    Path::new(raw)
        .components()
        .next()
        .is_some_and(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
}

/// Reject a `[source] path` that names no directory. An empty string
/// parses as the empty path, which `join` treats as a no-op, so the
/// executable would be looked up against whatever base happened to be
/// there -- the process cwd for `firmware-exec`. Name the refusal
/// instead of resolving a directory the manifest never declared.
fn reject_empty_source_path(origin: &Path, kind: &str, raw: &str) -> Result<(), ManifestError> {
    if raw.is_empty() {
        return Err(ManifestError::Parse {
            path: origin.to_path_buf(),
            message: format!(
                "source kind '{kind}' has an empty 'path'; an empty path names no \
                 directory and would leave the executable to be resolved against the \
                 process's current directory. State the directory holding it."
            ),
        });
    }
    Ok(())
}

/// Identity for a manifest-relative title: the directory the manifest
/// sits in, so `tests/micro/<name>/manifest.toml` yields `<name>`.
fn derive_content_id(origin: &Path) -> Option<String> {
    origin
        .parent()?
        .file_name()?
        .to_str()
        .map(std::string::ToString::to_string)
}

/// Translate a `[checkpoint]` table into its trigger.
///
/// # Errors
///
/// - [`ManifestError::UnknownCheckpointKind`] for a `kind` outside
///   `process-exit`, `first-rsx-write` and `pc`.
/// - [`ManifestError::BadCheckpointPc`] for kind `pc` with a missing
///   or unreadable address.
pub(super) fn parse_checkpoint(
    cp: &ManifestCheckpoint,
    origin: &Path,
) -> Result<CheckpointTrigger, ManifestError> {
    match cp.kind.as_str() {
        "process-exit" => Ok(CheckpointTrigger::ProcessExit),
        "first-rsx-write" => Ok(CheckpointTrigger::FirstRsxWrite),
        "pc" => {
            let raw = cp
                .pc
                .as_ref()
                .ok_or_else(|| ManifestError::BadCheckpointPc {
                    path: origin.to_path_buf(),
                    detail: "checkpoint kind 'pc' requires a 'pc = \"0xADDR\"' value".to_string(),
                })?;
            let parsed = parse_pc_literal(raw).map_err(|e| ManifestError::BadCheckpointPc {
                path: origin.to_path_buf(),
                detail: e.to_string(),
            })?;
            Ok(CheckpointTrigger::Pc(parsed))
        }
        other => Err(ManifestError::UnknownCheckpointKind {
            path: origin.to_path_buf(),
            kind: other.to_string(),
        }),
    }
}

/// Whether `[rsx] mirror` leaves `checkpoint` unable to fire.
///
/// The mirror makes the RSX region writable, so the put-pointer store
/// that `FirstRsxWrite` watches for cannot fault.
pub(super) fn mirror_makes_checkpoint_unreachable(
    rsx_mirror: bool,
    checkpoint: CheckpointTrigger,
) -> bool {
    rsx_mirror && matches!(checkpoint, CheckpointTrigger::FirstRsxWrite)
}

fn render_files_identical_hint(files_identical: bool) -> &'static str {
    if files_identical {
        " (files are byte-identical; one is likely a stray copy)"
    } else {
        ""
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
                let conflicting: Vec<&str> = ROOT_TABLE_KEYS
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
        let checkpoint = parse_checkpoint(&file.checkpoint, origin)?;
        let source_path = file.source.as_ref().and_then(|s| s.path.clone());
        let source = match file.source.as_ref().map(|s| s.kind.as_str()) {
            Some("disc") => GameSource::Disc,
            Some("hdd") => GameSource::Hdd,
            Some("firmware-exec") => {
                let dir = source_path.clone().ok_or_else(|| ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "source kind 'firmware-exec' requires a 'path = \"...\"' \
                              directory: a firmware executable is not installed under \
                              /dev_hdd0/game/, so there is no content-id directory to \
                              derive it from"
                        .to_string(),
                })?;
                reject_empty_source_path(origin, "firmware-exec", &dir)?;
                GameSource::FirmwareExec {
                    dir: PathBuf::from(dir),
                }
            }
            Some("manifest-relative") => {
                let rel = source_path.clone().ok_or_else(|| ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "source kind 'manifest-relative' requires a 'path = \"...\"' \
                              directory holding the executable, named relative to this \
                              manifest"
                        .to_string(),
                })?;
                reject_empty_source_path(origin, "manifest-relative", &rel)?;
                if discards_its_base(&rel) {
                    return Err(ManifestError::Parse {
                        path: origin.to_path_buf(),
                        message: format!(
                            "source kind 'manifest-relative' path {rel:?} is rooted or \
                             carries a drive prefix, so joining it onto the manifest's \
                             directory would discard that directory and the reference \
                             would not be manifest-relative at all. Name the directory \
                             relative to this manifest, or use kind = \"firmware-exec\" \
                             for a host-absolute one."
                        ),
                    });
                }
                GameSource::ManifestRelative {
                    dir: manifest_dir(origin).join(rel),
                }
            }
            Some(other) => {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: format!(
                        "unknown source kind '{other}' \
                         (accepted: disc, hdd, firmware-exec, manifest-relative)"
                    ),
                });
            }
            None => GameSource::Hdd,
        };
        if source_path.is_some()
            && !matches!(
                source,
                GameSource::FirmwareExec { .. } | GameSource::ManifestRelative { .. }
            )
        {
            return Err(ManifestError::Parse {
                path: origin.to_path_buf(),
                message: "[source] path is only meaningful for kind = \"firmware-exec\" \
                          or kind = \"manifest-relative\"; hdd and disc titles derive \
                          their directory from content_id"
                    .to_string(),
            });
        }
        let content_id = match file.title.content_id {
            Some(id) => id,
            None if matches!(source, GameSource::ManifestRelative { .. }) => {
                derive_content_id(origin).ok_or_else(|| ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "[title] content_id omitted and no directory name to derive \
                              it from; give the manifest a parent directory or state a \
                              content_id"
                        .to_string(),
                })?
            }
            None => {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "[title] content_id is required: it keys the registry and \
                              names the directory an hdd or disc title boots from. Only a \
                              manifest-relative title, which has no PSN identity, may omit it."
                        .to_string(),
                });
            }
        };
        let (rsx_mirror, rsx_consume) = file
            .rsx
            .as_ref()
            .map(|r| (r.mirror, r.consume))
            .unwrap_or((false, false));
        if mirror_makes_checkpoint_unreachable(rsx_mirror, checkpoint) {
            return Err(ManifestError::Parse {
                path: origin.to_path_buf(),
                message: "`[rsx] mirror = true` is incompatible with \
                          `checkpoint.kind = \"first-rsx-write\"`: the mirror \
                          makes the RSX region writable, so the put-pointer \
                          write that FirstRsxWrite watches for cannot fault."
                    .to_string(),
            });
        }
        if rsx_consume && !rsx_mirror {
            return Err(ManifestError::Parse {
                path: origin.to_path_buf(),
                message: "`[rsx] consume = true` requires `[rsx] mirror = true`: \
                          without the mirror the cursor never observes the guest's \
                          put-pointer stores, so the 40F honest consumer has nothing \
                          to walk. Enable mirror or remove consume."
                    .to_string(),
            });
        }
        let content = file.content.map(|c| ContentManifest {
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
        // A block with no files registers nothing, yet the provider
        // would still select a base for it and refuse a boot that has
        // none.
        if content.as_ref().is_some_and(|c| c.files.is_empty()) {
            return Err(ManifestError::Parse {
                path: origin.to_path_buf(),
                message: "[content] lists no files; drop the block or name the files it \
                          registers"
                    .to_string(),
            });
        }
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
        // Validate at load time so the error carries the manifest path.
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
        let distribution = Distribution::from_kebab(&file.title.distribution).ok_or_else(|| {
            ManifestError::Parse {
                path: origin.to_path_buf(),
                message: format!(
                    "unknown distribution {:?} \
                     (accepted: psn-hdd, retail-hdd, disc-iso, firmware-exec, microtest)",
                    file.title.distribution
                ),
            }
        })?;
        // A stale plaintext EBOOT.elf can drift from the on-disk SELF; force
        // the SCE-wrapped EBOOT.BIN ahead of EBOOT.elf when both are listed.
        if let (Some(elf_pos), Some(bin_pos)) = (
            file.title
                .eboot_candidates
                .iter()
                .position(|c| c == "EBOOT.elf"),
            file.title
                .eboot_candidates
                .iter()
                .position(|c| c == "EBOOT.BIN"),
        ) {
            if elf_pos < bin_pos {
                return Err(ManifestError::Parse {
                    path: origin.to_path_buf(),
                    message: "eboot_candidates lists EBOOT.elf before EBOOT.BIN; reorder so the \
                         SCE-wrapped EBOOT.BIN is tried first. A stale plaintext .elf in the \
                         USRDIR would otherwise shadow the canonical SELF."
                        .to_string(),
                });
            }
        }
        let matrix = matrix::build(
            file.bench.map(|b| b.matrix).unwrap_or_default(),
            file.title.system_ver.as_deref(),
            &source,
            rsx_mirror,
            origin,
        )?;
        Ok(TitleManifest {
            content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            year: file.title.year,
            developer: file.title.developer,
            engine: file.title.engine,
            distribution,
            rap_filename: file.title.rap_filename,
            bench_max_steps: file.title.bench_max_steps,
            system_ver: file.title.system_ver,
            checkpoint,
            source,
            rsx_mirror,
            rsx_consume,
            content,
            mounts,
            matrix,
        })
    }
}

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;
