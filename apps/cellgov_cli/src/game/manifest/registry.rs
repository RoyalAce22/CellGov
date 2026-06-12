//! Directory-of-manifests registry. Validates short-name and
//! content-id uniqueness across a scanned directory and exposes
//! lookup by either key.

use std::path::{Path, PathBuf};

use super::loader::ManifestError;
use super::model::TitleManifest;

/// Registry of titles loaded from a directory of TOML manifests.
/// Short name and content id are both unique keys.
#[derive(Debug, Clone, Default)]
pub struct TitleRegistry {
    manifests: Vec<TitleManifest>,
}

impl TitleRegistry {
    /// Load every `*.toml` under `dir`, validate short-name and
    /// content-id uniqueness, and sort by filename. A missing `dir`
    /// yields an empty registry; other I/O errors surface.
    pub fn scan_dir(dir: &Path) -> Result<Self, ManifestError> {
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
            // Skip dotfiles so backup/tooling `.backup.toml` does not load.
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

    /// Case-sensitive short-name lookup.
    pub fn by_short_name(&self, name: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.short_name == name)
    }

    pub fn by_content_id(&self, content_id: &str) -> Option<&TitleManifest> {
        self.manifests.iter().find(|m| m.content_id == content_id)
    }

    /// Comma-separated short names, or `"<none>"` when empty.
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

/// True iff both reads succeed and contents match; false on I/O
/// error. Short-circuits on `metadata` length.
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

#[cfg(test)]
#[path = "tests/registry_tests.rs"]
mod tests;
