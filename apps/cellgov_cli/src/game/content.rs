//! Boot-time content provider.
//!
//! Reads each entry of a [`ContentManifest`] off the host filesystem
//! and registers the bytes in [`Lv2Host::fs_store_mut`] under the
//! manifest's `guest_path`. Called once during [`super::boot::prepare`]
//! before the title's step loop runs, so any `sys_fs_open` the title
//! issues already sees the manifest-driven blobs.
//!
//! A missing host file is a startup error rather than a silent
//! ENOENT-at-runtime, per the design doc: a misconfigured manifest
//! must not look like a runtime FS bug.

use std::path::{Path, PathBuf};

use cellgov_lv2::{FsError, Lv2Host};

use super::manifest::ContentManifest;

/// Why [`register_content_blobs`] could not register a manifest's
/// content. Distinct variants let the caller surface the right
/// startup-error message: "fixture missing on disk" reads
/// differently than "two manifest entries collide on the same
/// guest path".
#[derive(Debug)]
pub enum ContentRegisterError {
    /// Reading the host file failed (most commonly NotFound, but
    /// permission-denied / IO errors land here too).
    HostFileRead {
        guest_path: String,
        host_path: PathBuf,
        source: std::io::Error,
        /// Name of the override env var that selected the base,
        /// when applicable. Surfaced in the Display impl so a
        /// developer who forgot to drop the file into their
        /// override dir sees the env var name in the error.
        override_env: Option<String>,
    },
    /// Two manifest entries name the same `guest_path`. Single-write
    /// blob registration forbids this.
    DuplicateGuestPath {
        guest_path: String,
        first_host_path: PathBuf,
        second_host_path: PathBuf,
    },
}

impl std::fmt::Display for ContentRegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostFileRead {
                guest_path,
                host_path,
                source,
                override_env,
            } => {
                write!(
                    f,
                    "content: failed to read host file {} for guest path {:?}: {source}",
                    host_path.display(),
                    guest_path,
                )?;
                if let Some(env) = override_env {
                    write!(
                        f,
                        " (override env var {env} is set; either drop the \
                         file into that directory or unset {env} to fall \
                         back to the manifest's checked-in base)",
                    )?;
                }
                Ok(())
            }
            Self::DuplicateGuestPath {
                guest_path,
                first_host_path,
                second_host_path,
            } => write!(
                f,
                "content: duplicate guest path {:?} in manifest \
                 (first host source {}, second host source {})",
                guest_path,
                first_host_path.display(),
                second_host_path.display(),
            ),
        }
    }
}

impl std::error::Error for ContentRegisterError {}

/// Resolve `path` against `base` if it is relative; absolute paths
/// pass through unchanged. Pure path arithmetic, no I/O.
fn resolve(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// Source of the resolved content base directory. Reported back
/// so the boot banner can announce where blobs were sourced from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBaseSource {
    /// Manifest's checked-in `base` (synthetic stubs, default).
    Manifest,
    /// EBOOT-relative USRDIR auto-discovered: every manifest entry
    /// resolved to an existing file under the title's USRDIR
    /// alongside the EBOOT being loaded. `path` carries the USRDIR
    /// for the boot banner.
    Usrdir { path: PathBuf },
    /// Override env var named by `[content] override_base_env` was
    /// set; `env` carries the var name so the boot banner can echo
    /// it.
    Override { env: String },
}

/// Whether every manifest entry resolves to an existing regular
/// file under `base`. Used as the soft probe for the USRDIR
/// resolution tier: a partial USRDIR (some files missing) falls
/// through to the manifest's checked-in base rather than failing
/// loud, so a developer with an incomplete local install still
/// gets the synthetic-stub fallback.
fn all_entries_resolve_under(base: &Path, manifest: &ContentManifest) -> bool {
    manifest.files.iter().all(|entry| {
        let p = resolve(base, &entry.host_path);
        std::fs::metadata(&p).map(|m| m.is_file()).unwrap_or(false)
    })
}

/// Read each manifest entry off disk and register the bytes in
/// `host.fs_store_mut`. Relative `host_path`s are resolved against
/// the resolved base; relative `base` is resolved against
/// `workspace_root`.
///
/// # Resolution priority
///
/// 1. `override_base` (when `Some`) -- explicit override via env
///    var. Hard-fail on any missing file with a diagnostic that
///    names the env var.
/// 2. `usrdir_base` (when `Some` AND every manifest entry resolves
///    under it) -- auto-discovered EBOOT-adjacent USRDIR. Soft
///    probe: if any file is missing the whole tier is skipped and
///    the resolution falls through to (3).
/// 3. `manifest.base` -- the checked-in fallback (synthetic stubs
///    in the public test suite). Hard-fail on any missing file.
///
/// # Errors
///
/// Surfaces on first failure; later entries are not attempted, and
/// the FsStore is left in whatever partial state earlier entries
/// produced. The caller (boot pipeline) treats this as a fatal
/// startup error.
pub fn register_content_blobs(
    manifest: &ContentManifest,
    workspace_root: &Path,
    override_base: Option<&Path>,
    usrdir_base: Option<&Path>,
    host: &mut Lv2Host,
) -> Result<ContentBaseSource, ContentRegisterError> {
    let (base, source) = if let Some(p) = override_base {
        // Even an override path may be relative; resolve against
        // workspace_root for parity with the manifest path.
        (
            resolve(workspace_root, &p.to_string_lossy()),
            ContentBaseSource::Override {
                env: manifest
                    .override_base_env
                    .clone()
                    .unwrap_or_else(|| "<no override_base_env declared>".to_string()),
            },
        )
    } else if let Some(usrdir) = usrdir_base.filter(|u| all_entries_resolve_under(u, manifest)) {
        (
            usrdir.to_path_buf(),
            ContentBaseSource::Usrdir {
                path: usrdir.to_path_buf(),
            },
        )
    } else {
        (
            resolve(workspace_root, &manifest.base),
            ContentBaseSource::Manifest,
        )
    };
    let override_env_for_err = match &source {
        ContentBaseSource::Override { env } => Some(env.clone()),
        ContentBaseSource::Manifest | ContentBaseSource::Usrdir { .. } => None,
    };
    for entry in &manifest.files {
        let host_path = resolve(&base, &entry.host_path);
        let bytes =
            std::fs::read(&host_path).map_err(|io_err| ContentRegisterError::HostFileRead {
                guest_path: entry.guest_path.clone(),
                host_path: host_path.clone(),
                source: io_err,
                override_env: override_env_for_err.clone(),
            })?;
        if let Err(FsError::PathAlreadyRegistered) = host
            .fs_store_mut()
            .register_blob(entry.guest_path.clone(), bytes)
        {
            // A duplicate guest_path within a single manifest is a
            // schema error, not a runtime issue. Surface the host
            // sources so the developer sees both colliding entries.
            // Find the prior host source by looking back through the
            // already-processed entries.
            let prior = manifest
                .files
                .iter()
                .find(|e| e.guest_path == entry.guest_path)
                .map(|e| resolve(&base, &e.host_path))
                .unwrap_or_else(|| host_path.clone());
            return Err(ContentRegisterError::DuplicateGuestPath {
                guest_path: entry.guest_path.clone(),
                first_host_path: prior,
                second_host_path: host_path,
            });
        }
    }
    Ok(source)
}

/// Look up the override base directory selected by a manifest's
/// `override_base_env`. Returns `Some(path)` when the env var is
/// set to a non-empty value; `None` otherwise (empty string,
/// absent variable, or no `override_base_env` declared). The
/// boot pipeline calls this and forwards the result to
/// [`register_content_blobs`].
///
/// Read from `getter` rather than directly from `std::env::var` so
/// tests can run without mutating process environment state.
pub fn override_base_from_env<F>(manifest: &ContentManifest, mut getter: F) -> Option<PathBuf>
where
    F: FnMut(&str) -> Option<String>,
{
    let env_name = manifest.override_base_env.as_deref()?;
    let value = getter(env_name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::manifest::ContentEntry;

    /// Tempdir with the same RAII shape as the manifest tests.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("cellgov_content_{name}_{}", std::process::id()));
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

    fn write_file(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn registers_all_entries_in_fs_store() {
        let tmp = TmpDir::new("happy_path");
        write_file(&tmp.path().join("first.xml"), b"<root/>");
        write_file(&tmp.path().join("Localization.xml"), b"<i18n/>");
        let manifest = ContentManifest {
            base: tmp.path().to_string_lossy().into_owned(),
            override_base_env: None,
            files: vec![
                ContentEntry {
                    guest_path: "/app_home/Data/Resources/first.xml".to_string(),
                    host_path: "first.xml".to_string(),
                },
                ContentEntry {
                    guest_path: "/app_home/Data/Local/Localization.xml".to_string(),
                    host_path: "Localization.xml".to_string(),
                },
            ],
        };
        let mut host = Lv2Host::new();
        register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host).unwrap();
        assert_eq!(host.fs_store().blob_count(), 2);
        assert_eq!(
            host.fs_store()
                .lookup_blob("/app_home/Data/Resources/first.xml"),
            Some(b"<root/>".as_slice()),
        );
        assert_eq!(
            host.fs_store()
                .lookup_blob("/app_home/Data/Local/Localization.xml"),
            Some(b"<i18n/>".as_slice()),
        );
    }

    #[test]
    fn missing_host_file_is_a_startup_error() {
        let tmp = TmpDir::new("missing");
        // Note: no file written.
        let manifest = ContentManifest {
            base: tmp.path().to_string_lossy().into_owned(),
            override_base_env: None,
            files: vec![ContentEntry {
                guest_path: "/p".to_string(),
                host_path: "absent.xml".to_string(),
            }],
        };
        let mut host = Lv2Host::new();
        let err = register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host)
            .expect_err("missing host file must surface");
        match err {
            ContentRegisterError::HostFileRead {
                guest_path,
                override_env,
                ..
            } => {
                assert_eq!(guest_path, "/p");
                assert!(
                    override_env.is_none(),
                    "no override env in play for the manifest-base case",
                );
            }
            other => panic!("expected HostFileRead, got {other}"),
        }
        // FsStore stays empty on failure.
        assert_eq!(host.fs_store().blob_count(), 0);
    }

    #[test]
    fn relative_base_is_resolved_against_workspace_root() {
        let tmp = TmpDir::new("rel_base");
        let workspace = tmp.path();
        write_file(&workspace.join("fx").join("first.xml"), b"<r/>");
        let manifest = ContentManifest {
            base: "fx".to_string(),
            override_base_env: None,
            files: vec![ContentEntry {
                guest_path: "/p/first.xml".to_string(),
                host_path: "first.xml".to_string(),
            }],
        };
        let mut host = Lv2Host::new();
        register_content_blobs(&manifest, workspace, None, None, &mut host).unwrap();
        assert_eq!(
            host.fs_store().lookup_blob("/p/first.xml"),
            Some(b"<r/>".as_slice())
        );
    }

    #[test]
    fn absolute_host_path_overrides_base() {
        let tmp = TmpDir::new("abs_host");
        write_file(&tmp.path().join("abs.xml"), b"<a/>");
        let manifest = ContentManifest {
            base: "/some/unrelated/base".to_string(),
            override_base_env: None,
            files: vec![ContentEntry {
                guest_path: "/p/abs.xml".to_string(),
                host_path: tmp.path().join("abs.xml").to_string_lossy().into_owned(),
            }],
        };
        let mut host = Lv2Host::new();
        register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host).unwrap();
        assert_eq!(
            host.fs_store().lookup_blob("/p/abs.xml"),
            Some(b"<a/>".as_slice())
        );
    }

    #[test]
    fn duplicate_guest_path_is_a_startup_error() {
        let tmp = TmpDir::new("dup");
        write_file(&tmp.path().join("a.xml"), b"a");
        write_file(&tmp.path().join("b.xml"), b"b");
        let manifest = ContentManifest {
            base: tmp.path().to_string_lossy().into_owned(),
            override_base_env: None,
            files: vec![
                ContentEntry {
                    guest_path: "/dup".to_string(),
                    host_path: "a.xml".to_string(),
                },
                ContentEntry {
                    guest_path: "/dup".to_string(),
                    host_path: "b.xml".to_string(),
                },
            ],
        };
        let mut host = Lv2Host::new();
        let err = register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host)
            .expect_err("duplicate guest_path must surface");
        match err {
            ContentRegisterError::DuplicateGuestPath { guest_path, .. } => {
                assert_eq!(guest_path, "/dup");
            }
            other => panic!("expected DuplicateGuestPath, got {other}"),
        }
        // The first entry registered before the failure; the
        // second's bytes did not land.
        assert_eq!(host.fs_store().blob_count(), 1);
        assert_eq!(host.fs_store().lookup_blob("/dup"), Some(b"a".as_slice()));
    }

    #[test]
    fn empty_files_list_succeeds_without_registering_anything() {
        let manifest = ContentManifest {
            base: ".".to_string(),
            override_base_env: None,
            files: vec![],
        };
        let mut host = Lv2Host::new();
        register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host).unwrap();
        assert_eq!(host.fs_store().blob_count(), 0);
    }

    #[test]
    fn override_base_replaces_manifest_base_when_files_present() {
        let synthetic = TmpDir::new("synth_overridden");
        let real = TmpDir::new("real_override");
        // Synthetic has stub content; real override has different
        // bytes; the override path wins.
        write_file(&synthetic.path().join("first.xml"), b"SYNTH");
        write_file(&real.path().join("first.xml"), b"REAL");
        let manifest = ContentManifest {
            base: synthetic.path().to_string_lossy().into_owned(),
            override_base_env: Some("DOES_NOT_MATTER_FOR_THIS_TEST".to_string()),
            files: vec![ContentEntry {
                guest_path: "/first.xml".to_string(),
                host_path: "first.xml".to_string(),
            }],
        };
        let mut host = Lv2Host::new();
        let source = register_content_blobs(
            &manifest,
            Path::new("/unused"),
            Some(real.path()),
            None,
            &mut host,
        )
        .unwrap();
        assert!(matches!(source, ContentBaseSource::Override { .. }));
        assert_eq!(
            host.fs_store().lookup_blob("/first.xml"),
            Some(b"REAL".as_slice()),
            "override path's bytes must be the ones registered",
        );
    }

    #[test]
    fn override_base_missing_file_error_carries_env_name() {
        // The override env is set, but the override directory does
        // not contain the named file. The diagnostic must name the
        // env var so the developer knows which knob to fix.
        let real = TmpDir::new("real_missing");
        let manifest = ContentManifest {
            base: "tests/fixtures/synthetic_unused".to_string(),
            override_base_env: Some("CELLGOV_TEST_OVERRIDE_DIR".to_string()),
            files: vec![ContentEntry {
                guest_path: "/p".to_string(),
                host_path: "absent.xml".to_string(),
            }],
        };
        let mut host = Lv2Host::new();
        let err = register_content_blobs(
            &manifest,
            Path::new("/unused"),
            Some(real.path()),
            None,
            &mut host,
        )
        .expect_err("missing override file must surface");
        // Render before destructuring so the Display impl can be
        // checked alongside the structural assertion.
        let msg = format!("{}", err);
        match err {
            ContentRegisterError::HostFileRead {
                override_env: Some(env),
                ..
            } => {
                assert_eq!(env, "CELLGOV_TEST_OVERRIDE_DIR");
                assert!(
                    msg.contains("CELLGOV_TEST_OVERRIDE_DIR"),
                    "Display must name the override env var, got: {msg}",
                );
            }
            other => panic!("expected HostFileRead with override_env Some, got {other:?}"),
        }
    }

    #[test]
    fn override_base_lookup_returns_none_when_env_unset() {
        let manifest = ContentManifest {
            base: "fx".to_string(),
            override_base_env: Some("UNSET_ENV_VAR_FOR_TEST".to_string()),
            files: vec![],
        };
        let result = override_base_from_env(&manifest, |_| None);
        assert!(result.is_none());
    }

    #[test]
    fn override_base_lookup_returns_none_for_empty_string() {
        // An exported-but-empty value is treated as unset; useful
        // for shell scripts that conditionally export.
        let manifest = ContentManifest {
            base: "fx".to_string(),
            override_base_env: Some("MAYBE_EMPTY".to_string()),
            files: vec![],
        };
        let result = override_base_from_env(&manifest, |name| {
            assert_eq!(name, "MAYBE_EMPTY");
            Some(String::new())
        });
        assert!(result.is_none());
    }

    #[test]
    fn override_base_lookup_returns_path_when_env_set() {
        let manifest = ContentManifest {
            base: "fx".to_string(),
            override_base_env: Some("SET_TO_PATH".to_string()),
            files: vec![],
        };
        let result = override_base_from_env(&manifest, |name| {
            assert_eq!(name, "SET_TO_PATH");
            Some("/tmp/local-flow".to_string())
        });
        assert_eq!(result, Some(PathBuf::from("/tmp/local-flow")));
    }

    #[test]
    fn override_base_lookup_returns_none_when_no_env_var_declared() {
        // Manifest without override_base_env never reads any env;
        // the getter must not be called for an undeclared override.
        let manifest = ContentManifest {
            base: "fx".to_string(),
            override_base_env: None,
            files: vec![],
        };
        let result = override_base_from_env(&manifest, |_| {
            panic!("getter must not be called when override_base_env is None")
        });
        assert!(result.is_none());
    }

    /// Build a manifest matching flOw's three-XML layout for the
    /// USRDIR-resolution tests. host_path values are nested under
    /// Data/.../X.xml so a real USRDIR (with the same nesting)
    /// resolves to existing files.
    fn flow_shaped_manifest(synthetic_base: &Path) -> ContentManifest {
        ContentManifest {
            base: synthetic_base.to_string_lossy().into_owned(),
            override_base_env: Some("CELLGOV_NPUA80001_CONTENT_DIR".to_string()),
            files: vec![
                ContentEntry {
                    guest_path: "/app_home/Data/Resources/first.xml".to_string(),
                    host_path: "Data/Resources/first.xml".to_string(),
                },
                ContentEntry {
                    guest_path: "/app_home/Data/Local/Localization.xml".to_string(),
                    host_path: "Data/Local/Localization.xml".to_string(),
                },
            ],
        }
    }

    #[test]
    fn usrdir_with_all_files_present_takes_priority_over_manifest_base() {
        let synth = TmpDir::new("usrdir_synth");
        let usrdir = TmpDir::new("usrdir_real");
        // Both bases populated; USRDIR has different bytes so we
        // can prove which one was used.
        write_file(&synth.path().join("Data/Resources/first.xml"), b"SYN");
        write_file(&synth.path().join("Data/Local/Localization.xml"), b"SYN");
        write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
        write_file(&usrdir.path().join("Data/Local/Localization.xml"), b"USR");
        let manifest = flow_shaped_manifest(synth.path());
        let mut host = Lv2Host::new();
        let source = register_content_blobs(
            &manifest,
            Path::new("/unused"),
            None,
            Some(usrdir.path()),
            &mut host,
        )
        .unwrap();
        assert!(matches!(source, ContentBaseSource::Usrdir { .. }));
        assert_eq!(
            host.fs_store()
                .lookup_blob("/app_home/Data/Resources/first.xml"),
            Some(b"USR".as_slice()),
            "USRDIR bytes must win when all entries resolve under it",
        );
    }

    #[test]
    fn partial_usrdir_falls_through_to_manifest_base() {
        // Pin the soft-probe contract: if even ONE manifest entry
        // is missing under USRDIR, the whole tier is skipped and
        // the synthetic base wins. A developer with an incomplete
        // local install still gets a working test run.
        let synth = TmpDir::new("partial_synth");
        let usrdir = TmpDir::new("partial_usrdir");
        write_file(&synth.path().join("Data/Resources/first.xml"), b"SYN");
        write_file(&synth.path().join("Data/Local/Localization.xml"), b"SYN");
        // Only one of the two files exists in USRDIR.
        write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
        let manifest = flow_shaped_manifest(synth.path());
        let mut host = Lv2Host::new();
        let source = register_content_blobs(
            &manifest,
            Path::new("/unused"),
            None,
            Some(usrdir.path()),
            &mut host,
        )
        .unwrap();
        assert_eq!(
            source,
            ContentBaseSource::Manifest,
            "partial USRDIR must fall through to manifest base",
        );
        // Both blobs sourced from the synthetic base.
        assert_eq!(
            host.fs_store()
                .lookup_blob("/app_home/Data/Resources/first.xml"),
            Some(b"SYN".as_slice()),
        );
    }

    #[test]
    fn override_takes_priority_over_usrdir() {
        // Pin the resolution order: env override beats USRDIR even
        // when USRDIR has all the files. The override is the
        // explicit "I know what I am doing" channel.
        let synth = TmpDir::new("prio_synth");
        let usrdir = TmpDir::new("prio_usrdir");
        let override_dir = TmpDir::new("prio_override");
        write_file(&synth.path().join("Data/Resources/first.xml"), b"SYN");
        write_file(&synth.path().join("Data/Local/Localization.xml"), b"SYN");
        write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
        write_file(&usrdir.path().join("Data/Local/Localization.xml"), b"USR");
        write_file(
            &override_dir.path().join("Data/Resources/first.xml"),
            b"OVR",
        );
        write_file(
            &override_dir.path().join("Data/Local/Localization.xml"),
            b"OVR",
        );
        let manifest = flow_shaped_manifest(synth.path());
        let mut host = Lv2Host::new();
        let source = register_content_blobs(
            &manifest,
            Path::new("/unused"),
            Some(override_dir.path()),
            Some(usrdir.path()),
            &mut host,
        )
        .unwrap();
        assert!(matches!(source, ContentBaseSource::Override { .. }));
        assert_eq!(
            host.fs_store()
                .lookup_blob("/app_home/Data/Resources/first.xml"),
            Some(b"OVR".as_slice()),
        );
    }

    #[test]
    fn usrdir_none_uses_manifest_base() {
        // No EBOOT-relative USRDIR available (e.g., bench-boot
        // without a real EBOOT path); the manifest base wins.
        let synth = TmpDir::new("usrdir_none_synth");
        write_file(&synth.path().join("Data/Resources/first.xml"), b"SYN");
        write_file(&synth.path().join("Data/Local/Localization.xml"), b"SYN");
        let manifest = flow_shaped_manifest(synth.path());
        let mut host = Lv2Host::new();
        let source =
            register_content_blobs(&manifest, Path::new("/unused"), None, None, &mut host).unwrap();
        assert_eq!(source, ContentBaseSource::Manifest);
    }
}
