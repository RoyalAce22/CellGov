//! In-memory data model for a parsed title manifest. Wire format lives in
//! [`super::schema`]; TOML -> model translation lives in [`super::loader`].

use std::path::{Path, PathBuf};

use super::checkpoint::CheckpointTrigger;

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
        }
    }

    /// Kebab-case wire form used in TOML `distribution = "..."` fields.
    pub fn kebab_label(self) -> &'static str {
        match self {
            Self::PsnHdd => "psn-hdd",
            Self::RetailHdd => "retail-hdd",
            Self::DiscIso => "disc-iso",
            Self::FirmwareExec => "firmware-exec",
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

/// One title's manifest as loaded from `docs/title_manifests/<content-id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleManifest {
    /// PSN content id; primary lookup key and the directory name
    /// under `/dev_hdd0/game/`.
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
    /// (license 3) NPDRM titles that use `NP_KLIC_FREE`.
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

/// Why [`TitleManifest::resolve_eboot`] could not return a path.
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
    /// No candidate executable exists under the resolved USRDIR.
    /// `probe_errors` collects non-NotFound I/O errors.
    #[error("{}", render_not_found(searched, candidates, probe_errors))]
    NotFound {
        searched: PathBuf,
        candidates: Vec<String>,
        probe_errors: Vec<(PathBuf, std::io::Error)>,
    },
    /// `content_id` begins with `.`, which would resolve a hidden /
    /// in-progress directory (e.g. an install's `.staging-*` sibling)
    /// instead of a real title. There is no VFS directory scan -- titles
    /// come from explicit manifests -- so this resolver is the gate that
    /// keeps a dot-prefixed key off the boot path.
    #[error("title '{short_name}' has a hidden content-id {content_id:?} (leading '.')")]
    HiddenContentId {
        content_id: String,
        short_name: String,
    },
}

/// Test witness: counts how many times the hidden-content-id guard in
/// [`TitleManifest::resolve_eboot`] fired, so a test can prove the
/// guard executed rather than passing vacuously.
#[cfg(test)]
pub(crate) static HIDDEN_CONTENT_ID_REJECTIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn render_not_found(
    searched: &Path,
    candidates: &[String],
    probe_errors: &[(PathBuf, std::io::Error)],
) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("no executable found; looked for:");
    for name in candidates {
        let _ = write!(s, "\n  {}", searched.join(name).display());
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

    pub fn rsx_mirror(&self) -> bool {
        self.rsx_mirror
    }

    /// 40F honest FIFO consumer opt-in; see field doc.
    pub fn rsx_consume(&self) -> bool {
        self.rsx_consume
    }

    /// Return the first [`TitleManifest::eboot_candidates`] filename
    /// that exists as a regular file under the title's USRDIR.
    ///
    /// # Errors
    ///
    /// See [`ResolveEbootError`].
    pub fn resolve_eboot(&self, vfs_root: &Path) -> Result<PathBuf, ResolveEbootError> {
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
        let usrdir = match &self.source {
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
            GameSource::FirmwareExec { dir } => dir.clone(),
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

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;
