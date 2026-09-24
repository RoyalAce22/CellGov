//! Storing an artifact: the store error, the create-new write and the portable path.

use std::path::Path;

use super::finding::FuzzFindingArtifact;
use super::schema::ArtifactReduction;

impl FuzzFindingArtifact {
    /// Writes this artifact to `path`, creating the directory, unless
    /// the path already holds it.
    ///
    /// The write is create-new. A file already at `path` that describes
    /// the same finding stands: a rerun under another execution policy
    /// or campaign range produces the same finding. A finding's identity
    /// excludes its reduction state, so a reduced rerun matches a stored
    /// unreduced file; that reduction is not stored, and the refusal says
    /// so.
    ///
    /// # Errors
    ///
    /// [`ArtifactStoreError`] for a path with no parent, an encoding or
    /// write failure, different evidence already at the path, or a
    /// reduction the stored file does not hold.
    pub fn store(&self, path: &Path) -> Result<(), ArtifactStoreError> {
        let parent = path.parent().ok_or(ArtifactStoreError::NoParent)?;
        let write = |source| ArtifactStoreError::Write {
            path: path.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(parent).map_err(write)?;
        let encoded = serde_json::to_vec_pretty(self).map_err(ArtifactStoreError::Encoding)?;
        match create_new(path, &encoded) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = std::fs::read_to_string(path).map_err(write)?;
                match Self::parse_json(&existing) {
                    Ok(stored) if stored.describes_same_finding(self) => {
                        if self.reduction != ArtifactReduction::NotAttempted
                            && stored.reduction != self.reduction
                        {
                            Err(ArtifactStoreError::ReductionNotStored {
                                path: path.to_path_buf(),
                                stored: stored.reduction,
                            })
                        } else {
                            Ok(())
                        }
                    }
                    _ => Err(ArtifactStoreError::Collision {
                        path: path.to_path_buf(),
                    }),
                }
            }
            Err(source) => Err(write(source)),
        }
    }
}

/// Why [`FuzzFindingArtifact::store`] stored nothing.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactStoreError {
    /// The artifact path names no directory to write into.
    #[error("artifact path has no parent")]
    NoParent,
    /// The artifact did not encode as JSON.
    #[error("artifact encoding failed: {0}")]
    Encoding(#[source] serde_json::Error),
    /// The file system refused the directory, the write or the read-back.
    #[error("artifact write {} failed: {source}", path.display())]
    Write {
        /// The artifact path.
        path: std::path::PathBuf,
        /// The file-system refusal.
        #[source]
        source: std::io::Error,
    },
    /// The path already holds different finding evidence.
    #[error("artifact path {} already holds different evidence", path.display())]
    Collision {
        /// The artifact path.
        path: std::path::PathBuf,
    },
    /// The path already holds this finding under another reduction,
    /// which the store did not replace.
    #[error("artifact path {} already holds this finding with reduction {stored:?}", path.display())]
    ReductionNotStored {
        /// The artifact path.
        path: std::path::PathBuf,
        /// The reduction the stored file holds.
        stored: ArtifactReduction,
    },
}

/// Writes `bytes` to a file that must not exist yet, and syncs it.
///
/// # Errors
///
/// The file-system refusal, [`std::io::ErrorKind::AlreadyExists`] among
/// them.
pub(crate) fn create_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The portable path the store writes a finding to: `dir` as the
/// caller spelled it with trailing separators trimmed, one forward
/// slash, then `<stem>-<case_index>-<index>.json`.
///
/// A forward slash opens the file on every host, so the path doubles as
/// the replay command's text.
pub fn artifact_path(dir: &str, stem: &str, case_index: u64, index: u64) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}/{stem}-{case_index}-{index}.json",
        dir.trim_end_matches(['/', '\\'])
    ))
}
