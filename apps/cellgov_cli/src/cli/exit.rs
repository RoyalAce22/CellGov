//! Process-exit and whole-file-read helpers shared across every
//! CLI subcommand.

use std::path::{Path, PathBuf};

use cellgov_install::npdrm::NpdHeaderInfo;
use cellgov_install::sce::SceError;
use cellgov_ps3_abi::sce::SCE_MAGIC;

use crate::game::manifest::TitleManifest;

/// Print `msg` to stderr and exit with status 1.
pub(crate) fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

/// Read a file or die with a context-rich error.
pub(crate) fn load_file_or_die(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| die(&format!("failed to read {path}: {e}")))
}

/// Plaintext-ize a PPU image: pass non-SCE bytes (plaintext ELF /
/// PRX) through unchanged, and decrypt an SCE/SELF wrapper.
///
/// NPDRM titles resolve their klicensee from the RAP at
/// `<vfs_root>/home/00000001/exdata/<content_id>.rap` -- the same
/// exdata layout RPCS3 reads on boot, so a once-installed RAP is
/// found by content id with no per-invocation `--rap` or `--title`.
/// An absent RAP returns `None`: license-3 (free) titles fall back
/// to `NP_KLIC_FREE`, Network / Local titles surface
/// `NoRapForNpdrmTitle`. `path` is used only in diagnostics.
pub(crate) fn decrypt_ppu_self_or_die(bytes: &[u8], path: &str, vfs_root: &Path) -> Vec<u8> {
    if !(bytes.len() >= 4 && bytes[..4] == SCE_MAGIC) {
        return bytes.to_vec();
    }
    let exdata = vfs_root.join("home").join("00000001").join("exdata");
    let resolver = |npd: &NpdHeaderInfo| -> Option<[u8; 16]> {
        let rap_path = exdata.join(format!("{}.rap", npd.content_id));
        let rap_bytes = std::fs::read(&rap_path).ok()?;
        let rap_arr: [u8; 16] = rap_bytes.as_slice().try_into().unwrap_or_else(|_| {
            die(&format!(
                "RAP file {} is {} bytes; expected exactly 16",
                rap_path.display(),
                rap_bytes.len(),
            ))
        });
        Some(cellgov_install::npdrm::rap_to_klic(&rap_arr))
    };
    match cellgov_install::npdrm::decrypt_self_to_elf_auto(bytes, resolver) {
        Ok(elf) => elf,
        Err(e @ SceError::NoRapForNpdrmTitle { .. }) => die(&format!(
            "{e}; expected its RAP at {}/<content_id>.rap",
            exdata.display()
        )),
        Err(e) => die(&format!("failed to decrypt SELF {path}: {e}")),
    }
}

#[derive(Debug, thiserror::Error)]
enum LoadCandidateError {
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decrypt(#[from] SceError),
    #[error("bytes are not a SELF or plaintext ELF")]
    NotElf,
}

/// RAP path layout is `<vfs_root>/home/00000001/exdata/<rap>`.
fn klicensee_resolver(
    title: &TitleManifest,
    vfs_root: PathBuf,
) -> impl Fn(&NpdHeaderInfo) -> Option<[u8; 16]> {
    let rap_filename = title.rap_filename.clone();
    move |npd: &NpdHeaderInfo| -> Option<[u8; 16]> {
        // license 3 (free) falls back to NP_KLIC_FREE downstream when None.
        let rap_filename = rap_filename.as_ref()?;
        let rap_path = vfs_root
            .join("home")
            .join("00000001")
            .join("exdata")
            .join(rap_filename);
        let rap_bytes = match std::fs::read(&rap_path) {
            Ok(b) => b,
            Err(e) => die(&format!(
                "failed to read RAP for NPDRM title {} (license {}) at {}: {}",
                npd.content_id,
                npd.license as u32,
                rap_path.display(),
                e,
            )),
        };
        let rap_arr: [u8; 16] = rap_bytes.as_slice().try_into().unwrap_or_else(|_| {
            die(&format!(
                "RAP file {} is {} bytes; expected exactly 16",
                rap_path.display(),
                rap_bytes.len(),
            ))
        });
        Some(cellgov_install::npdrm::rap_to_klic(&rap_arr))
    }
}

/// Plaintext ELF bytes plus the boot identity read from the SELF
/// wrapper before decryption. Both identity fields are `None` for
/// raw-ELF inputs, which have no SELF headers; `control_flags1` is
/// also `None` for a SELF that carries no plaintext capability
/// header, which is the unprivileged case.
pub(crate) struct LoadedPpuImage {
    pub elf_data: Vec<u8>,
    pub authority_id: Option<u64>,
    pub control_flags1: Option<u32>,
}

/// Read a PPU image at an explicit path, resolving the klicensee for
/// NPDRM titles from the manifest's `rap_filename`.
pub(crate) fn load_ppu_image_with_title_or_die(
    path: &str,
    title: &TitleManifest,
    vfs_root: &Path,
) -> LoadedPpuImage {
    let bytes = load_file_or_die(path);
    if !(bytes.len() >= 4 && bytes[..4] == SCE_MAGIC) {
        return LoadedPpuImage {
            elf_data: bytes,
            authority_id: None,
            control_flags1: None,
        };
    }
    let authority_id = cellgov_install::sce::parse_program_authority_id(&bytes)
        .map_err(|e| die(&format!("SELF {path}: identification header: {e}")))
        .ok();
    let control_flags1 = cellgov_install::sce::parse_control_flags1(&bytes)
        .unwrap_or_else(|e| die(&format!("SELF {path}: capability header: {e}")));
    let resolver = klicensee_resolver(title, vfs_root.to_path_buf());
    let elf_data = cellgov_install::npdrm::decrypt_self_to_elf_auto(&bytes, resolver)
        .unwrap_or_else(|e| die(&format!("failed to decrypt SELF {path}: {e}")));
    LoadedPpuImage {
        elf_data,
        authority_id,
        control_flags1,
    }
}

/// Walk `eboot_candidates` in declaration order, returning the first
/// plaintext ELF that loads. The die-message enumerates each
/// candidate's typed cause when every candidate fails.
pub(crate) fn load_ppu_image_walk_candidates_or_die(
    title: &TitleManifest,
    vfs_root: &Path,
) -> (LoadedPpuImage, PathBuf) {
    let resolved = title.resolve_eboot(vfs_root).unwrap_or_else(|e| {
        // No content directory at all: the dump is not on this machine.
        eprintln!(
            "{} title={} (no content directory)",
            cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL,
            title.name()
        );
        die(&format!(
            "load ppu image: resolve_eboot for title {}: {}",
            title.name(),
            e,
        ))
    });
    let usrdir = resolved
        .parent()
        .unwrap_or_else(|| die("load ppu image: resolved EBOOT has no parent directory"))
        .to_path_buf();

    let resolver = klicensee_resolver(title, vfs_root.to_path_buf());
    let mut attempts: Vec<(String, LoadCandidateError)> = Vec::new();
    for candidate in &title.eboot_candidates {
        let path = usrdir.join(candidate);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                attempts.push((candidate.clone(), LoadCandidateError::Io(e)));
                continue;
            }
        };
        if bytes.len() >= 4 && bytes[..4] == SCE_MAGIC {
            let authority_id = cellgov_install::sce::parse_program_authority_id(&bytes).ok();
            let control_flags1 = cellgov_install::sce::parse_control_flags1(&bytes)
                .ok()
                .flatten();
            match cellgov_install::npdrm::decrypt_self_to_elf_auto(&bytes, &resolver) {
                Ok(elf) => {
                    return (
                        LoadedPpuImage {
                            elf_data: elf,
                            authority_id,
                            control_flags1,
                        },
                        path,
                    )
                }
                Err(e) => {
                    attempts.push((candidate.clone(), LoadCandidateError::Decrypt(e)));
                    continue;
                }
            }
        } else if bytes.len() >= 4 && bytes[..4] == [0x7F, b'E', b'L', b'F'] {
            return (
                LoadedPpuImage {
                    elf_data: bytes,
                    authority_id: None,
                    control_flags1: None,
                },
                path,
            );
        } else {
            attempts.push((candidate.clone(), LoadCandidateError::NotElf));
            continue;
        }
    }
    // Missing dump vs broken dump: only when every candidate failed
    // because the file does not exist is the title "not installed".
    // An existing file that failed to decrypt or parse must die
    // without the marker so the suites surface it as a boot failure.
    let all_missing = attempts.iter().all(|(_, why)| {
        matches!(why, LoadCandidateError::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
    });
    if all_missing {
        eprintln!(
            "{} title={} (no eboot candidate present)",
            cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL,
            title.name()
        );
    }
    let usrdir_str = usrdir.display();
    let attempts_str = attempts
        .iter()
        .map(|(name, why)| format!("    {name}: {why}"))
        .collect::<Vec<_>>()
        .join("\n");
    die(&format!(
        "load ppu image: every eboot_candidate for title {} failed under {usrdir_str}:\n{attempts_str}",
        title.name(),
    ))
}

#[cfg(test)]
#[path = "tests/exit_tests.rs"]
mod tests;
