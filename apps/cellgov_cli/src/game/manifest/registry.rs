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
mod tests {
    use super::*;
    use crate::game::manifest::test_fixtures::{TmpDir, FIRST_RSX_WRITE_TOML, PROCESS_EXIT_TOML};

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
    fn known_names_csv_empty_registry_is_labelled() {
        let reg = TitleRegistry::default();
        assert_eq!(reg.known_names_csv(), "<none>");
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
