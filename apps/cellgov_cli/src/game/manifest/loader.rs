//! TOML -> [`TitleManifest`] translation.

use std::path::{Path, PathBuf};

use super::checkpoint::{parse_pc_literal, CheckpointTrigger};
use super::model::{
    ContentEntry, ContentManifest, Distribution, GameSource, MountEntry, TitleManifest,
};
use super::schema::ManifestFile;

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
        let (rsx_mirror, rsx_consume) = file
            .rsx
            .as_ref()
            .map(|r| (r.mirror, r.consume))
            .unwrap_or((false, false));
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
                    "unknown distribution {:?} (accepted: psn-hdd, retail-hdd, disc-iso)",
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
        Ok(TitleManifest {
            content_id: file.title.content_id,
            short_name: file.title.short_name,
            display_name: file.title.display_name,
            eboot_candidates: file.title.eboot_candidates,
            year: file.title.year,
            developer: file.title.developer,
            engine: file.title.engine,
            distribution,
            rap_filename: file.title.rap_filename,
            checkpoint,
            source,
            rsx_mirror,
            rsx_consume,
            content,
            mounts,
        })
    }
}

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;
