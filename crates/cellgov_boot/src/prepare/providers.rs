//! The guest-visible content a boot registers before the title's first
//! instruction -- EBOOT-sibling images, manifest content blobs, and
//! mounts.

use std::collections::BTreeMap;

use cellgov_core::Runtime;
use cellgov_lv2::FsError;

use super::types::{DiagnosticOptions, TitleOptions};
use crate::content::{ContentBaseSource, ContentRegisterError};
use crate::mounts::MountRegisterError;

/// Why a boot's guest-visible content could not be registered.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// An EBOOT sibling exists and does not read.
    #[error("boot run: cannot read {path}: {source}")]
    SiblingRead {
        /// The sibling's host path.
        path: String,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// An override env var holds a value that is not Unicode.
    #[error(
        "override env var {name} is set to a value that is not Unicode ({raw:?}); \
         set it to a path or unset it"
    )]
    OverrideNotUnicode {
        /// The variable that was set.
        name: String,
        /// Its raw value.
        raw: std::ffi::OsString,
    },
    /// The working directory content and mount paths resolve against
    /// could not be read.
    #[error("cannot read CWD for {purpose}: {source}")]
    WorkingDirectory {
        /// What the directory was needed for.
        purpose: &'static str,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// A manifest content blob could not be registered.
    #[error("content provider failed: {0}")]
    Content(#[from] ContentRegisterError),
    /// A manifest mount could not be registered.
    #[error("mount provider failed: {0}")]
    Mount(#[from] MountRegisterError),
    /// A store-composed mount could not be registered.
    #[error("composed mount provider failed: {0}")]
    ComposedMount(#[from] ComposedMountError),
}

/// Resolves `sysSpuImageOpen("/app_home/spu_main.elf")` against an
/// EBOOT sibling; same discovery for a spawn microtest's child SELF.
///
/// # Errors
///
/// A sibling that exists and does not read.
pub(super) fn register_sibling_images(
    rt: &mut Runtime,
    elf_path: &str,
) -> Result<(), ProviderError> {
    let Some(parent) = eboot_dir(elf_path) else {
        return Ok(());
    };
    for (sibling, guest_path) in [
        ("spu_main.elf", b"/app_home/spu_main.elf".as_slice()),
        ("child.self", b"/app_home/child.self".as_slice()),
    ] {
        let candidate = parent.join(sibling);
        if candidate.exists() {
            let bytes = std::fs::read(&candidate).map_err(|source| ProviderError::SiblingRead {
                path: candidate.display().to_string(),
                source,
            })?;
            rt.lv2_host_mut()
                .content_store_mut()
                .register(guest_path, bytes);
        }
    }
    Ok(())
}

/// The directory the EBOOT sits in: the default base for manifest
/// content and the default host for a mount that declares none.
///
/// The loader opens a bare filename from the process cwd, so its
/// directory is `.`. `Path::parent` spells that directory as the empty
/// path, which does not canonicalize as a mount root. An empty or
/// root-only path yields `None`.
fn eboot_dir(elf_path: &str) -> Option<&std::path::Path> {
    let parent = std::path::Path::new(elf_path).parent()?;
    Some(if parent.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        parent
    })
}

/// The roots of a mount that declares no host and the bases of a
/// `[content]` entry, in shadowing order.
///
/// - An executable the candidate walk found sits in `eboot_dirs`, so
///   that list is the answer. A selected update's directory leads and
///   the base's follows: the shadowing the composed game mount applies.
/// - An explicit executable outside `eboot_dirs` (a build outside the
///   store) keeps its own directory alone. The composition's
///   directories describe the store's executable, which this boot does
///   not run, and one of them may not exist.
fn usrdir_bases(elf_path: &str, eboot_dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    match eboot_dir(elf_path) {
        Some(dir) if !eboot_dirs.iter().any(|b| same_directory(b, dir)) => vec![dir.to_path_buf()],
        _ => eboot_dirs.to_vec(),
    }
}

/// Whether two spellings name one directory.
///
/// The operator spells an explicit executable, so it can name a
/// composed directory another way (a relative path, `..`, a symlink);
/// the canonical forms settle that when both resolve. When either does
/// not resolve, the component-wise comparison decides; on Windows it
/// folds `/` and `\`.
fn same_directory(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Read every override env var the manifest declares, before either
/// provider runs.
///
/// # Errors
///
/// A declared variable is set to a value that is not Unicode.
fn read_override_env<I>(names: I) -> Result<BTreeMap<String, String>, ProviderError>
where
    I: IntoIterator<Item = String>,
{
    collect_override_env(names, |name| std::env::var(name))
}

/// The name-by-name half of [`read_override_env`].
///
/// An absent variable stays out of the map. Both consumers read an
/// empty value as a declared blank, so an empty entry for an absent
/// variable would change which base a manifest resolves to.
///
/// # Errors
///
/// A declared variable is set to a value that is not Unicode.
fn collect_override_env<I, F>(
    names: I,
    mut read: F,
) -> Result<BTreeMap<String, String>, ProviderError>
where
    I: IntoIterator<Item = String>,
    F: FnMut(&str) -> Result<String, std::env::VarError>,
{
    let mut values = BTreeMap::new();
    for name in names {
        match read(&name) {
            Ok(v) => {
                values.insert(name, v);
            }
            Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(raw)) => {
                return Err(ProviderError::OverrideNotUnicode { name, raw })
            }
        }
    }
    Ok(values)
}

/// Registers the manifest's content blobs; base selection lives on
/// [`crate::content::register_content_blobs`].
///
/// # Errors
///
/// The working directory does not read, an override env var is not
/// Unicode, or a blob does not register.
pub(super) fn register_content(
    rt: &mut Runtime,
    title: &TitleOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
) -> Result<(), ProviderError> {
    let Some(content) = title.manifest.content.as_ref() else {
        return Ok(());
    };
    let workspace_root =
        std::env::current_dir().map_err(|source| ProviderError::WorkingDirectory {
            purpose: "content base resolution",
            source,
        })?;
    let env = read_override_env(content.override_base_env.clone())?;
    let override_base =
        crate::content::override_base_from_env(content, |name| env.get(name).cloned());
    let source = crate::content::register_content_blobs(
        content,
        &workspace_root,
        override_base.as_deref(),
        &usrdir_bases(title.elf_path, title.eboot_dirs),
        rt.lv2_host_mut(),
    )?;
    if diagnostics.print_banner {
        let label = content_source_label(&source, override_base.as_deref());
        sink.note(&format!(
            "content: registered {} blob(s) from {label}",
            content.files.len(),
        ));
    }
    Ok(())
}

/// Runs after [`register_content`] so the FsStore path-existence check
/// wins over mount resolution.
///
/// # Errors
///
/// A composed or manifest mount the table refuses, a working directory
/// that does not read, or an override env var that is not Unicode.
pub(super) fn register_mounts(
    rt: &mut Runtime,
    title: &TitleOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
) -> Result<(), ProviderError> {
    rt.lv2_host_mut()
        .fs_mounts_mut()
        .set_files(std::rc::Rc::new(crate::mounts::HostMountFiles));
    register_composed_mounts(rt, title, diagnostics, sink)?;
    if title.manifest.mounts.is_empty() {
        return Ok(());
    }
    let workspace_root =
        std::env::current_dir().map_err(|source| ProviderError::WorkingDirectory {
            purpose: "mount path resolution",
            source,
        })?;
    let env = read_override_env(
        title
            .manifest
            .mounts
            .iter()
            .filter_map(|m| m.override_env.clone()),
    )?;
    let n = crate::mounts::register_mounts(
        &title.manifest.mounts,
        &workspace_root,
        &usrdir_bases(title.elf_path, title.eboot_dirs),
        |name| env.get(name).cloned(),
        rt.lv2_host_mut(),
    )?;
    if diagnostics.print_banner {
        sink.note(&format!("mounts: registered {n} mount(s)"));
    }
    Ok(())
}

/// Why a store-composed mount could not be registered.
#[derive(Debug, thiserror::Error)]
pub enum ComposedMountError {
    /// The mount table refused the prefix, or the root list was empty.
    #[error(
        "composition names mount prefix {prefix:?} over {roots} root(s), which the mount \
         table refuses"
    )]
    Rejected {
        /// The guest prefix the composition named.
        prefix: String,
        /// An empty root list is refused here too, so the count
        /// separates that case from a bad prefix.
        roots: usize,
    },
    /// Another mount already answers this prefix.
    #[error("mount prefix {prefix:?}: {source}")]
    NotAdded {
        /// The guest prefix the composition named.
        prefix: String,
        #[source]
        source: FsError,
    },
}

/// Runs before the manifest's own mounts so a manifest prefix that
/// encloses a composed one cannot answer first.
fn register_composed_mounts(
    rt: &mut Runtime,
    title: &TitleOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
) -> Result<(), ProviderError> {
    for mount in title.composed_mounts {
        let registered =
            cellgov_lv2::FsMount::with_roots(mount.prefix.clone(), mount.roots.clone())
                .ok_or_else(|| ComposedMountError::Rejected {
                    prefix: mount.prefix.clone(),
                    roots: mount.roots.len(),
                })
                .and_then(|m| {
                    rt.lv2_host_mut().fs_mounts_mut().add(m).map_err(|source| {
                        ComposedMountError::NotAdded {
                            prefix: mount.prefix.clone(),
                            source,
                        }
                    })
                });
        registered?;
    }
    if diagnostics.print_banner && !title.composed_mounts.is_empty() {
        sink.note(&format!(
            "mounts: composed {} store mount(s)",
            title.composed_mounts.len()
        ));
    }
    Ok(())
}

fn content_source_label(
    source: &ContentBaseSource,
    override_base: Option<&std::path::Path>,
) -> String {
    match source {
        ContentBaseSource::Usrdir { paths } => format!(
            "EBOOT directories ({})",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ContentBaseSource::Override { env } => format!(
            "override env {env}={}",
            override_base
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
    }
}

#[cfg(test)]
#[path = "tests/providers_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/providers_bases_tests.rs"]
mod bases_tests;

#[cfg(test)]
#[path = "tests/providers_override_env_tests.rs"]
mod override_env_tests;
