//! The guest-visible content a boot registers before the title's first
//! instruction -- EBOOT-sibling images, manifest content blobs, and
//! mounts.

use cellgov_core::Runtime;
use cellgov_lv2::FsError;

use super::types::PrepareOptions;
use crate::cli::exit::die;

/// Resolves `sysSpuImageOpen("/app_home/spu_main.elf")` against an
/// EBOOT sibling; same discovery for a spawn microtest's child SELF.
pub(super) fn register_sibling_images(rt: &mut Runtime, elf_path: &str) {
    let Some(parent) = eboot_dir(elf_path) else {
        return;
    };
    for (sibling, guest_path) in [
        ("spu_main.elf", b"/app_home/spu_main.elf".as_slice()),
        ("child.self", b"/app_home/child.self".as_slice()),
    ] {
        let candidate = parent.join(sibling);
        if candidate.exists() {
            let bytes = std::fs::read(&candidate).unwrap_or_else(|e| {
                die(&format!(
                    "boot run: cannot read {}: {e}",
                    candidate.display()
                ))
            });
            rt.lv2_host_mut()
                .content_store_mut()
                .register(guest_path, bytes);
        }
    }
}

/// The directory the EBOOT sits in: the default base for manifest
/// content and the default host for a mount that declares none.
///
/// The loader opens a bare filename from the process cwd (see
/// `cli::exit::load_file_or_die`), so its directory is `.`.
/// `Path::parent` spells that directory as the empty path, which
/// does not canonicalize as a mount root. An empty or root-only path
/// yields `None`.
fn eboot_dir(elf_path: &str) -> Option<&std::path::Path> {
    let parent = std::path::Path::new(elf_path).parent()?;
    Some(if parent.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        parent
    })
}

/// The override env var's value, or `None` when it is unset.
///
/// A value that is not Unicode stops the boot with an error that
/// names the var.
fn env_override(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) => Some(v),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(raw)) => die(&format!(
            "override env var {name} is set to a value that is not Unicode ({raw:?}); \
             set it to a path or unset it"
        )),
    }
}

/// Registers the manifest's content blobs; base selection lives on
/// [`crate::game::content::register_content_blobs`].
pub(super) fn register_content(rt: &mut Runtime, opts: &PrepareOptions<'_>) {
    let Some(content) = opts.title.content.as_ref() else {
        return;
    };
    let workspace_root = std::env::current_dir()
        .unwrap_or_else(|e| die(&format!("cannot read CWD for content base resolution: {e}")));
    let override_base = crate::game::content::override_base_from_env(content, env_override);
    let registration_result = crate::game::content::register_content_blobs(
        content,
        &workspace_root,
        override_base.as_deref(),
        eboot_dir(opts.elf_path),
        rt.lv2_host_mut(),
    );
    match registration_result {
        Ok(source) => {
            if opts.print_banner {
                let label = content_source_label(&source, override_base.as_deref());
                println!(
                    "content: registered {} blob(s) from {label}",
                    content.files.len(),
                );
            }
        }
        Err(e) => die(&format!("content provider failed: {e}")),
    }
}

/// Runs after [`register_content`] so the FsStore path-existence check
/// wins over mount resolution.
pub(super) fn register_mounts(rt: &mut Runtime, opts: &PrepareOptions<'_>) {
    register_composed_mounts(rt, opts);
    if opts.title.mounts.is_empty() {
        return;
    }
    let workspace_root = std::env::current_dir()
        .unwrap_or_else(|e| die(&format!("cannot read CWD for mount path resolution: {e}")));
    let n = match crate::game::mounts::register_mounts(
        &opts.title.mounts,
        &workspace_root,
        eboot_dir(opts.elf_path),
        env_override,
        rt.lv2_host_mut(),
    ) {
        Ok(n) => n,
        Err(e) => die(&format!("mount provider failed: {e}")),
    };
    if opts.print_banner {
        println!("mounts: registered {n} mount(s)");
    }
}

/// Why a store-composed mount could not be registered.
#[derive(Debug, thiserror::Error)]
enum ComposedMountError {
    /// The mount table refused the prefix, or the root list was empty.
    #[error(
        "composition names mount prefix {prefix:?} over {roots} root(s), which the mount \
         table refuses"
    )]
    Rejected {
        prefix: String,
        /// An empty root list is refused here too, so the count
        /// separates that case from a bad prefix.
        roots: usize,
    },
    /// Another mount already answers this prefix.
    #[error("mount prefix {prefix:?}: {source}")]
    NotAdded {
        prefix: String,
        #[source]
        source: FsError,
    },
}

/// Runs before the manifest's own mounts so a manifest prefix that
/// encloses a composed one cannot answer first.
fn register_composed_mounts(rt: &mut Runtime, opts: &PrepareOptions<'_>) {
    for mount in opts.composed_mounts {
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
        if let Err(e) = registered {
            die(&format!("composed mount provider failed: {e}"));
        }
    }
    if opts.print_banner && !opts.composed_mounts.is_empty() {
        println!(
            "mounts: composed {} store mount(s)",
            opts.composed_mounts.len()
        );
    }
}

fn content_source_label(
    source: &crate::game::content::ContentBaseSource,
    override_base: Option<&std::path::Path>,
) -> String {
    use crate::game::content::ContentBaseSource;
    match source {
        ContentBaseSource::Usrdir { path } => {
            format!("EBOOT directory ({})", path.display())
        }
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
