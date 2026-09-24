//! The directory walk and per-file ingest.

use std::path::{Path, PathBuf};

use crate::keys::hex::value_bytes;
use crate::keys::names::{classify_name, NameClass};
use crate::keys::{IgnoreReason, KeyVaultError};

use super::builder::{Loader, PendingHalf};
use super::text::file_at;

/// Largest file the directory walk reads; bigger ones are listed as
/// ignored unread.
const MAX_KEY_FILE_BYTES: u64 = 1024 * 1024;

/// Directory depth the vault walk descends below the root.
const MAX_WALK_DEPTH: usize = 4;

impl Loader {
    pub(in crate::keys) fn walk_dir(
        &mut self,
        dir: &Path,
        depth: usize,
    ) -> Result<(), KeyVaultError> {
        let io = |source| KeyVaultError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()
            .map_err(io)?;
        entries.sort();
        for path in entries {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if hidden {
                self.vault.ignore(file_at(&path), IgnoreReason::Hidden);
                continue;
            }
            if path.is_dir() {
                if depth < MAX_WALK_DEPTH {
                    self.walk_dir(&path, depth + 1)?;
                } else {
                    self.vault.ignore(
                        file_at(&path),
                        IgnoreReason::TooDeep {
                            depth: depth + 1,
                            max: MAX_WALK_DEPTH,
                        },
                    );
                }
            } else {
                self.ingest_file(&path)?;
            }
        }
        Ok(())
    }

    pub(in crate::keys) fn ingest_file(&mut self, path: &Path) -> Result<(), KeyVaultError> {
        let io = |source| KeyVaultError::Io {
            path: path.to_path_buf(),
            source,
        };
        let at = file_at(path);
        if let Some(reason) = skip_reason(&extension_of(path)) {
            self.vault.ignore(at, reason);
            return Ok(());
        }
        let len = std::fs::metadata(path).map_err(io)?.len();
        if len > MAX_KEY_FILE_BYTES {
            self.vault.ignore(at, IgnoreReason::TooLarge { bytes: len });
            return Ok(());
        }
        let bytes = std::fs::read(path).map_err(io)?;
        self.ingest_bytes(path, &bytes)
    }

    pub(in crate::keys) fn ingest_bytes(
        &mut self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), KeyVaultError> {
        let at = file_at(path);
        let extension = extension_of(path);
        // The host reads a dotted version in a per-key name
        // (`lv2-key-3.60-3.61`) as a numeric extension; the name is whole.
        let name_of = if !extension.is_empty() && extension.bytes().all(|b| b.is_ascii_digit()) {
            Path::file_name
        } else {
            Path::file_stem
        };
        let stem = name_of(path)
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        self.vault.note_source(path);
        if extension_of(path) == "toml" {
            return self.ingest_toml(at, bytes);
        }
        // A per-key file holds one value and nothing else; anything
        // with another shape is a keyfile to read line by line.
        let value = value_bytes(bytes);
        let one_value = matches!(value.len(), 0x10 | 0x20 | 0x40);
        let by_name = if one_value {
            classify_name(&stem, Some(value.len()))
        } else {
            NameClass::Unknown
        };
        match by_name {
            NameClass::Scalar(slot) => self.add_scalar(slot, value, &stem, at),
            NameClass::Half { kind, part, label } => self.add_half(
                kind,
                part,
                label,
                PendingHalf {
                    bytes: value,
                    what: stem,
                    at,
                },
                true,
            ),
            NameClass::Unknown => match std::str::from_utf8(bytes) {
                Ok(text) => {
                    // A file that adds nothing -- no key, no half, no
                    // set-aside entry -- still leaves one line in the
                    // report, so "read and empty" is told apart from
                    // "read and placed".
                    let before = (self.vault.summary(), self.pending.len());
                    self.ingest_text(path, text)?;
                    if before == (self.vault.summary(), self.pending.len()) {
                        self.vault.ignore(at, IgnoreReason::NothingFound);
                    }
                    Ok(())
                }
                Err(_) => {
                    self.vault.ignore(at, IgnoreReason::NotText);
                    Ok(())
                }
            },
        }
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Extensions the directory walk never reads.
fn skip_reason(ext: &str) -> Option<IgnoreReason> {
    match ext {
        "rap" => Some(IgnoreReason::RapFile),
        "zip" | "7z" | "rar" | "gz" | "tar" | "pdf" | "png" | "jpg" | "jpeg" | "gif" | "html"
        | "htm" | "iso" | "pkg" | "pup" | "self" | "sprx" | "prx" | "elf" | "exe" | "dll"
        | "so" | "dylib" | "bak" | "tmp" => Some(IgnoreReason::UnsupportedFile {
            extension: ext.to_string(),
        }),
        _ => None,
    }
}
