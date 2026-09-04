//! In-memory data model for a parsed title manifest. Wire format lives in
//! [`super::schema`]; TOML -> model translation lives in [`super::loader`].

use std::path::{Path, PathBuf};

use super::checkpoint::CheckpointTrigger;
use super::matrix::{CellKey, MatrixCell};

/// How the title's executable is located on disk. Defaults to `Hdd`
/// when `[source]` is omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameSource {
    /// EBOOT at `<vfs>/game/<content-id>/USRDIR/`.
    Hdd,
    /// EBOOT at `<vfs-parent>/dev_bdvd/<content-id>/PS3_GAME/USRDIR/`.
    /// Requires `vfs_root` to have a non-empty parent.
    Disc,
    /// Executable shipped inside the firmware image, at `dir` on the
    /// host. Ignores `vfs_root`: a firmware executable is not
    /// installed under `/dev_hdd0/game/` and has no content id
    /// directory. Relative `dir` resolves against the process's
    /// current directory, matching [`MountEntry::host`].
    FirmwareExec { dir: PathBuf },
    /// Executable sitting beside the manifest that names it, at `dir`
    /// on the host. Ignores `vfs_root`, like [`Self::FirmwareExec`],
    /// but the loader has already joined the declared path onto the
    /// manifest's own directory, so the reference resolves the same
    /// from any working directory. The loader rejects a rooted or
    /// drive-prefixed declared path, which `Path::join` would drop
    /// that directory for.
    ManifestRelative { dir: PathBuf },
}

/// Distribution channel for the `titles.md` Format column. Display only;
/// runtime mount semantics live on [`GameSource`]. Two wire forms:
/// kebab-case (`"psn-hdd"`) in TOML, title-case (`"PSN HDD"`) in the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum Distribution {
    PsnHdd,
    RetailHdd,
    DiscIso,
    /// Shipped inside the firmware image rather than distributed as a
    /// title (vsh and the other CoreOS executables).
    FirmwareExec,
    /// Built from in-repo source under `tests/micro/`; never
    /// distributed. Keeps a structural microtest out of the retail
    /// channels it would otherwise have to claim.
    Microtest,
}

impl Distribution {
    /// Matrix Format-column label (title case with spaces).
    #[allow(dead_code, reason = "consumed by titles-gen tests")]
    pub fn format_label(self) -> &'static str {
        match self {
            Self::PsnHdd => "PSN HDD",
            Self::RetailHdd => "Retail HDD",
            Self::DiscIso => "Disc ISO",
            Self::FirmwareExec => "Firmware Exec",
            Self::Microtest => "Microtest",
        }
    }

    /// Kebab-case wire form used in TOML `distribution = "..."` fields.
    pub fn kebab_label(self) -> &'static str {
        match self {
            Self::PsnHdd => "psn-hdd",
            Self::RetailHdd => "retail-hdd",
            Self::DiscIso => "disc-iso",
            Self::FirmwareExec => "firmware-exec",
            Self::Microtest => "microtest",
        }
    }

    /// Inverse of [`Self::kebab_label`].
    pub fn from_kebab(s: &str) -> Option<Self> {
        use strum::VariantArray;
        Self::VARIANTS
            .iter()
            .find(|v| v.kebab_label() == s)
            .copied()
    }
}

/// One title's manifest as loaded from `titles/<content-id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleManifest {
    /// PSN content id; primary lookup key and, for
    /// [`GameSource::Hdd`] / [`GameSource::Disc`], the directory name
    /// holding the executable. A
    /// [`GameSource::ManifestRelative`] title has no PSN identity and
    /// may omit it in TOML; the loader then fills it from the
    /// manifest's own directory name.
    pub content_id: String,
    /// Short CLI name for `--title <name>`. Unique across the registry.
    pub short_name: String,
    pub display_name: String,
    /// Executable filenames tried in priority order under USRDIR.
    pub eboot_candidates: Vec<String>,
    /// Year of first release; displayed in the `titles.md` matrix.
    pub year: u16,
    pub developer: String,
    pub engine: String,
    /// Distribution channel for the matrix's Format column.
    pub distribution: Distribution,
    /// RAP filename under `<vfs_root>/home/00000001/exdata/` for
    /// NPDRM titles. Required when `EBOOT.BIN` is NPDRM-wrapped
    /// (license 1/2); omitted for APP-keyed disc titles and free
    /// (license 3) NPDRM titles, which use the vault's free klicensee.
    pub rap_filename: Option<String>,
    /// Instruction cap the witness suite boots this title under.
    /// `None` uses the recorder's 100M-instruction default.
    pub bench_max_steps: Option<u64>,
    /// Built-in boot checkpoint; CLI `--checkpoint` overrides.
    pub checkpoint: CheckpointTrigger,
    pub source: GameSource,
    /// Mutually exclusive with `CheckpointTrigger::FirstRsxWrite`:
    /// a writable region cannot fault on the put-pointer store.
    pub rsx_mirror: bool,
    /// 40F honest FIFO consumer opt-in. Requires `rsx_mirror = true`;
    /// `consume = true, mirror = false` is rejected at load time.
    pub rsx_consume: bool,
    pub content: Option<ContentManifest>,
    /// Mount-table registration order matches declaration order;
    /// the dispatch layer consults mounts in that order on a miss.
    pub mounts: Vec<MountEntry>,
    /// The `(firmware, game version)` cells this title declares, in
    /// declaration order. A non-empty matrix marks exactly one cell as
    /// the reference.
    pub matrix: Vec<MatrixCell>,
}

/// One mount-table entry. `prefix` must start with `/`. `override_env`,
/// when set non-empty, replaces `host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub prefix: String,
    pub host: String,
    pub override_env: Option<String>,
}

/// Per-title content provider; entries map a guest path to a host
/// file registered in `Lv2Host::fs_store` at boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentManifest {
    /// Base for relative `host_path`s; relative resolves against
    /// the workspace root.
    pub base: String,
    /// When set non-empty in the process env, replaces [`Self::base`].
    pub override_base_env: Option<String>,
    pub files: Vec<ContentEntry>,
}

/// `guest_path` is what `sys_fs_open` sees; `host_path` is the
/// on-disk source (relative paths resolve against
/// [`ContentManifest::base`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentEntry {
    pub guest_path: String,
    pub host_path: String,
}

/// Why an EBOOT could not be found for a title.
#[derive(Debug, thiserror::Error)]
pub enum ResolveEbootError {
    /// Disc title with a `vfs_root` that has no non-empty parent.
    #[error(
        "disc title '{short_name}' needs vfs-root with a parent directory (got {})",
        vfs_root.display()
    )]
    MisconfiguredVfsRoot {
        vfs_root: PathBuf,
        short_name: String,
    },
    /// No candidate executable exists under any searched directory.
    /// `probe_errors` collects non-NotFound I/O errors.
    #[error(
        "{}",
        render_not_found(searched, candidates, probe_errors, not_regular)
    )]
    NotFound {
        /// Directories probed, in probe order.
        searched: Vec<PathBuf>,
        candidates: Vec<String>,
        probe_errors: Vec<(PathBuf, std::io::Error)>,
        /// Candidates that exist but are not regular files -- a
        /// directory or a special file sitting on the name. Reported
        /// apart from a plain miss: the name is taken, so the fix is
        /// a different one.
        not_regular: Vec<PathBuf>,
    },
    /// `content_id` begins with `.`. For [`GameSource::Hdd`] and
    /// [`GameSource::Disc`] that would resolve a hidden / in-progress
    /// directory (e.g. an install's `.staging-*` sibling) instead of a
    /// real title. There is no VFS directory scan -- titles come from
    /// explicit manifests -- so this resolver is the gate.
    ///
    /// It refuses ahead of the source match, so it also covers the two
    /// sources whose executable path never mentions the id: the id
    /// still names the title's `tests/fixtures/<content-id>/` anchor
    /// directory, which is source-independent, and a
    /// [`GameSource::ManifestRelative`] title's id is derived from a
    /// directory name rather than chosen, so it is the one that can
    /// pick up a leading dot without anyone writing one.
    #[error("title '{short_name}' has a hidden content-id {content_id:?} (leading '.')")]
    HiddenContentId {
        content_id: String,
        short_name: String,
    },
}

/// Test witness: counts how many times the hidden-content-id guard in
/// [`TitleManifest::eboot_dirs`] fired, so a test can prove the
/// guard executed rather than passing vacuously. Shared by every test
/// in the process, so a reader compares for growth, not for an exact
/// delta.
#[cfg(test)]
pub(crate) static HIDDEN_CONTENT_ID_REJECTIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn render_not_found(
    searched: &[PathBuf],
    candidates: &[String],
    probe_errors: &[(PathBuf, std::io::Error)],
    not_regular: &[PathBuf],
) -> String {
    use std::fmt::Write as _;
    // The two lists feed one cross product, so an empty list leaves the
    // "looked for" list empty -- a diagnostic that names nothing.
    let mut s = if searched.is_empty() {
        String::from("no executable found; no directory was given to probe")
    } else if candidates.is_empty() {
        let dirs: Vec<String> = searched.iter().map(|d| d.display().to_string()).collect();
        format!(
            "no executable found; the title lists no eboot_candidates to probe under {}",
            dirs.join(", ")
        )
    } else {
        let mut s = String::from("no executable found; looked for:");
        for dir in searched {
            for name in candidates {
                let _ = write!(s, "\n  {}", dir.join(name).display());
            }
        }
        s
    };
    for p in not_regular {
        let _ = write!(s, "\n  exists but is not a regular file: {}", p.display());
    }
    for (p, e) in probe_errors {
        let _ = write!(s, "\n  probe error: {}: {e}", p.display());
    }
    s
}

impl TitleManifest {
    pub fn name(&self) -> &str {
        &self.short_name
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn checkpoint_trigger(&self) -> CheckpointTrigger {
        self.checkpoint
    }

    /// The cell the headline row renders, or `None` when the title
    /// declares no cells.
    ///
    /// # Panics
    ///
    /// Panics in a debug build if the matrix marks more than one cell
    /// as the reference.
    pub fn reference_cell(&self) -> Option<&MatrixCell> {
        // The loader enforces the one-reference rule, but this struct is
        // constructible without it. Picking the first of two marked
        // cells would name a configuration nobody chose.
        debug_assert!(
            self.matrix.iter().filter(|c| c.reference).count() <= 1,
            "title '{}' carries more than one reference cell",
            self.short_name
        );
        self.matrix.iter().find(|c| c.reference)
    }

    pub fn cell(&self, key: &CellKey) -> Option<&MatrixCell> {
        self.matrix.iter().find(|c| c.key == *key)
    }

    pub fn rsx_mirror(&self) -> bool {
        self.rsx_mirror
    }

    /// 40F honest FIFO consumer opt-in; see field doc.
    pub fn rsx_consume(&self) -> bool {
        self.rsx_consume
    }

    /// The directory the title's executable sits in, derived from the
    /// VFS root alone.
    ///
    /// This derivation covers a title the versioned store does not
    /// hold. A stored title takes its directories from its install
    /// records.
    ///
    /// # Errors
    ///
    /// - [`ResolveEbootError::HiddenContentId`]
    /// - [`ResolveEbootError::MisconfiguredVfsRoot`]
    pub fn eboot_dirs(&self, vfs_root: &Path) -> Result<Vec<PathBuf>, ResolveEbootError> {
        // A dot-prefixed content-id would resolve a hidden directory
        // (an in-progress `.staging-*` / `.uninstalling-*` sibling);
        // reject it so such residue can never be booted.
        if self.content_id.starts_with('.') {
            #[cfg(test)]
            HIDDEN_CONTENT_ID_REJECTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Err(ResolveEbootError::HiddenContentId {
                content_id: self.content_id.clone(),
                short_name: self.short_name.clone(),
            });
        }
        Ok(vec![match &self.source {
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
            GameSource::FirmwareExec { dir } | GameSource::ManifestRelative { dir } => dir.clone(),
        }])
    }

    /// Return the first [`TitleManifest::eboot_candidates`] filename
    /// that exists as a regular file under `dirs`, in order.
    ///
    /// An earlier directory shadows a later one, so a selected
    /// update's executable wins over the base's.
    ///
    /// # Errors
    ///
    /// [`ResolveEbootError::NotFound`], which names every path probed.
    pub fn resolve_eboot_in(&self, dirs: &[PathBuf]) -> Result<PathBuf, ResolveEbootError> {
        let mut probe_errors = Vec::new();
        let mut not_regular = Vec::new();
        for dir in dirs {
            for name in &self.eboot_candidates {
                let p = dir.join(name);
                match std::fs::metadata(&p) {
                    Ok(md) if md.is_file() => return Ok(p),
                    // The name is taken by a directory or a special
                    // file. Not a miss -- say so rather than folding it
                    // into the "looked for" list, which reads as
                    // "nothing was there".
                    Ok(_) => not_regular.push(p),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => probe_errors.push((p, e)),
                }
            }
        }
        Err(ResolveEbootError::NotFound {
            searched: dirs.to_vec(),
            candidates: self.eboot_candidates.clone(),
            probe_errors,
            not_regular,
        })
    }
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;
