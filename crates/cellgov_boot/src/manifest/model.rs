//! In-memory data model for a parsed title manifest. Wire format lives in
//! [`super::schema`]; TOML -> model translation lives in [`super::loader`].

use std::path::{Path, PathBuf};

use cellgov_install::store::{DISC_DISTRIBUTION, PSN_HDD_DISTRIBUTION};
use cellgov_ps3_abi::format::hdd0::GAME_DIR;
use cellgov_ps3_abi::format::title_tree::{BDVD_MOUNT, DISC_GAME_DIR, USRDIR};

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
    FirmwareExec {
        /// Host directory holding the executable.
        dir: PathBuf,
    },
    /// Executable sitting beside the manifest that names it, at `dir`
    /// on the host. Ignores `vfs_root`, like [`Self::FirmwareExec`],
    /// but the loader has already joined the declared path onto the
    /// manifest's own directory, so the reference resolves the same
    /// from any working directory. The loader rejects a rooted or
    /// drive-prefixed declared path, which `Path::join` would drop
    /// that directory for.
    ManifestRelative {
        /// Host directory holding the executable, already joined onto
        /// the manifest's own directory.
        dir: PathBuf,
    },
}

/// Distribution channel for the `titles.md` Format column. Display only;
/// runtime mount semantics live on [`GameSource`]. Two wire forms:
/// kebab-case (`"psn-hdd"`) in TOML, title-case (`"PSN HDD"`) in the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum Distribution {
    /// Downloaded from PSN and installed under `/dev_hdd0/game/`.
    PsnHdd,
    /// A retail disc's title installed to the HDD.
    RetailHdd,
    /// Read from a disc image.
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
    /// The two an install record also carries are the record's own tags.
    pub fn kebab_label(self) -> &'static str {
        match self {
            Self::PsnHdd => PSN_HDD_DISTRIBUTION,
            Self::RetailHdd => "retail-hdd",
            Self::DiscIso => DISC_DISTRIBUTION,
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

/// One title's manifest as loaded from `title_manifests/<content-id>.toml`.
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
    /// Name the matrix and the boot banner show.
    pub display_name: String,
    /// Executable filenames tried in priority order under USRDIR.
    pub eboot_candidates: Vec<String>,
    /// Year of first release; displayed in the `titles.md` matrix.
    pub year: u16,
    /// Studio credited for the title.
    pub developer: String,
    /// Engine the title runs on.
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
    /// The title's floor: the firmware its own PARAM.SFO requires, as a
    /// store version key. It is present on every title with a PARAM.SFO
    /// (an hdd or disc source) and absent on one without
    /// (firmware-shipped or manifest-relative). It derives
    /// [`Self::reference_key`].
    pub system_ver: Option<String>,
    /// Built-in boot checkpoint; CLI `--checkpoint` overrides.
    pub checkpoint: CheckpointTrigger,
    /// Where the executable and the title's content live.
    pub source: GameSource,
    /// Mutually exclusive with `CheckpointTrigger::FirstRsxWrite`:
    /// a writable region cannot fault on the put-pointer store.
    pub rsx_mirror: bool,
    /// 40F honest FIFO consumer opt-in. Requires `rsx_mirror = true`;
    /// `consume = true, mirror = false` is rejected at load time.
    pub rsx_consume: bool,
    /// Files the boot registers in the LV2 host's store.
    pub content: Option<ContentManifest>,
    /// Mount-table registration order matches declaration order;
    /// the dispatch layer consults mounts in that order on a miss.
    pub mounts: Vec<MountEntry>,
    /// The `(firmware, game version)` cells this title declares: the
    /// cell [`Self::system_ver`] derives first, then every
    /// `[[bench.matrix]]` row in declaration order. A row that repeats
    /// the derived cell attaches its override to it and adds no entry.
    pub matrix: Vec<MatrixCell>,
}

/// One mount-table entry. `prefix` must start with `/`. `override_env`,
/// when set non-empty, replaces `host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// Guest path prefix the mount answers.
    pub prefix: String,
    /// Host directory the prefix maps to; `None` is the directory the
    /// EBOOT sits in, which is where a PSN title's `/app_home` lives.
    pub host: Option<String>,
    /// Env var whose non-empty value replaces `host`.
    pub override_env: Option<String>,
}

/// Per-title content provider; entries map a guest path to a host
/// file registered in `Lv2Host::fs_store` at boot.
///
/// The boot selects the base for a relative `host_path`; see
/// `crate::content::register_content_blobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentManifest {
    /// Env var whose non-empty value names the content base directory
    /// in place of the EBOOT's own.
    pub override_base_env: Option<String>,
    /// One entry per file the boot registers.
    pub files: Vec<ContentEntry>,
}

/// `guest_path` is what `sys_fs_open` sees; `host_path` is the
/// on-disk source (a relative path resolves against the base the boot
/// selects; see [`ContentManifest`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentEntry {
    /// Path `sys_fs_open` sees.
    pub guest_path: String,
    /// On-disk source, absolute or resolved against the selected base.
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
        /// The root the caller named.
        vfs_root: PathBuf,
        /// The title that needs a parent of it.
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
        /// Executable filenames tried in each directory.
        candidates: Vec<String>,
        /// Probes that failed for a reason other than absence.
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
        /// The id with the leading dot.
        content_id: String,
        /// The title that carries it.
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

/// Instruction cap a cell is measured under when neither it nor its
/// title declares one.
pub const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

impl TitleManifest {
    /// The short name `--title` takes.
    pub fn name(&self) -> &str {
        &self.short_name
    }

    /// Whether the title ships inside the firmware image, so its
    /// version axis is the firmware's and it has no store entry.
    #[must_use]
    pub fn ships_in_firmware(&self) -> bool {
        matches!(self.source, GameSource::FirmwareExec { .. })
    }

    /// The name the matrix and the boot banner show.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Where a run of this title should stop.
    pub fn checkpoint_trigger(&self) -> CheckpointTrigger {
        self.checkpoint
    }

    /// The cell the headline row renders: the title's floor times its
    /// base install. It is `None` for a title with no PARAM.SFO.
    pub fn reference_key(&self) -> Option<CellKey> {
        self.system_ver.as_deref().map(super::matrix::derived_key)
    }

    /// The declared cell `key` names, when the title declares one.
    pub fn cell(&self, key: &CellKey) -> Option<&MatrixCell> {
        self.matrix.iter().find(|c| c.key == *key)
    }

    /// Where a run of `cell` stops: the cell's own checkpoint, else the
    /// title's. `dev record-anchors` records an anchor at this stop
    /// condition, and the gate holds a run there.
    pub fn cell_checkpoint(&self, cell: Option<&MatrixCell>) -> CheckpointTrigger {
        cell.and_then(|c| c.checkpoint)
            .unwrap_or_else(|| self.checkpoint_trigger())
    }

    /// The instruction cap a run of `cell` is recorded and gated under:
    /// the cell's own, else the title's, else
    /// [`DEFAULT_BENCH_MAX_STEPS`].
    #[must_use]
    pub fn cell_max_steps(&self, cell: Option<&MatrixCell>) -> u64 {
        cell.and_then(|c| c.bench_max_steps)
            .or(self.bench_max_steps)
            .unwrap_or(DEFAULT_BENCH_MAX_STEPS)
    }

    /// Whether the RSX region is writable for this title; see field doc.
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
            GameSource::Hdd => vfs_root.join(GAME_DIR).join(&self.content_id).join(USRDIR),
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
                    .join(BDVD_MOUNT)
                    .join(&self.content_id)
                    .join(DISC_GAME_DIR)
                    .join(USRDIR)
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

#[cfg(test)]
#[path = "tests/cell_fallback_tests.rs"]
mod cell_fallback_tests;
