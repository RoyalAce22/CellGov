//! The guest-visible content a boot registers before the title's first
//! instruction -- EBOOT-sibling images, manifest content blobs, and
//! mounts.

use cellgov_core::Runtime;

use super::types::PrepareOptions;
use crate::cli::exit::die;

/// Resolves `sysSpuImageOpen("/app_home/spu_main.elf")` against an
/// EBOOT sibling; same discovery for a spawn microtest's child SELF.
pub(super) fn register_sibling_images(rt: &mut Runtime, elf_path: &str) {
    let Some(parent) = std::path::Path::new(elf_path).parent() else {
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
                    "run-game: cannot read {}: {e}",
                    candidate.display()
                ))
            });
            rt.lv2_host_mut()
                .content_store_mut()
                .register(guest_path, bytes);
        }
    }
}

/// Content-base resolution priority, high to low: the override env
/// var, then EBOOT-relative USRDIR auto-discovery, then the manifest's
/// checked-in base.
pub(super) fn register_content(rt: &mut Runtime, opts: &PrepareOptions<'_>) {
    let Some(content) = opts.title.content.as_ref() else {
        return;
    };
    let workspace_root = std::env::current_dir()
        .unwrap_or_else(|e| die(&format!("cannot read CWD for content base resolution: {e}")));
    let override_base =
        crate::game::content::override_base_from_env(content, |name| std::env::var(name).ok());
    let usrdir_base = std::path::Path::new(opts.elf_path).parent();
    let registration_result = crate::game::content::register_content_blobs(
        content,
        &workspace_root,
        override_base.as_deref(),
        usrdir_base,
        rt.lv2_host_mut(),
    );
    match registration_result {
        Ok(source) => {
            if opts.print_banner {
                let label = content_source_label(&source, &content.base, override_base.as_deref());
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
    if opts.title.mounts.is_empty() {
        return;
    }
    let workspace_root = std::env::current_dir()
        .unwrap_or_else(|e| die(&format!("cannot read CWD for mount path resolution: {e}")));
    let n = match crate::game::mounts::register_mounts(
        &opts.title.mounts,
        &workspace_root,
        |name| std::env::var(name).ok(),
        rt.lv2_host_mut(),
    ) {
        Ok(n) => n,
        Err(e) => die(&format!("mount provider failed: {e}")),
    };
    if opts.print_banner {
        println!("mounts: registered {n} mount(s)");
    }
}

fn content_source_label(
    source: &crate::game::content::ContentBaseSource,
    manifest_base: &str,
    override_base: Option<&std::path::Path>,
) -> String {
    use crate::game::content::ContentBaseSource;
    match source {
        ContentBaseSource::Manifest => format!("manifest base ({manifest_base})"),
        ContentBaseSource::Usrdir { path } => {
            format!("EBOOT-adjacent USRDIR ({})", path.display())
        }
        ContentBaseSource::Override { env } => format!(
            "override env {env}={}",
            override_base
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
    }
}
