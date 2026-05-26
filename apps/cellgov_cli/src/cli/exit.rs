//! Process-exit and whole-file-read helpers shared across every
//! CLI subcommand.

use std::path::{Path, PathBuf};

use cellgov_firmware::sce::{NpdHeaderInfo, SceError};
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

/// Why one specific candidate failed to load. The candidate-walking
/// loop preserves the typed cause per candidate.
#[derive(Debug, thiserror::Error)]
enum LoadCandidateError {
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decrypt(#[from] SceError),
    #[error("bytes are not a SELF or plaintext ELF")]
    NotElf,
}

/// Build the NPDRM klicensee resolver from a title manifest's
/// `rap_filename` field. RAP path layout is
/// `<vfs_root>/home/00000001/exdata/<rap>`.
fn klicensee_resolver(
    title: &TitleManifest,
    vfs_root: PathBuf,
) -> impl Fn(&NpdHeaderInfo) -> Option<[u8; 16]> {
    let rap_filename = title.rap_filename.clone();
    move |npd: &NpdHeaderInfo| -> Option<[u8; 16]> {
        // license 3 (free) substitutes NP_KLIC_FREE inside
        // decrypt_self_to_elf_auto if the resolver returns None.
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
                npd.license,
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
        Some(cellgov_firmware::npdrm::rap_to_klic(&rap_arr))
    }
}

/// Read a PPU image at an explicit path, resolving the klicensee for
/// NPDRM titles from the manifest's `rap_filename`. Dies with a
/// context-rich error on read or decrypt failure.
pub(crate) fn load_ppu_image_with_title_or_die(
    path: &str,
    title: &TitleManifest,
    vfs_root: &Path,
) -> Vec<u8> {
    let bytes = load_file_or_die(path);
    if !(bytes.len() >= 4 && bytes[..4] == SCE_MAGIC) {
        return bytes;
    }
    let resolver = klicensee_resolver(title, vfs_root.to_path_buf());
    cellgov_firmware::npdrm::decrypt_self_to_elf_auto(&bytes, resolver)
        .unwrap_or_else(|e| die(&format!("failed to decrypt SELF {path}: {e}")))
}

/// Walk `eboot_candidates` in declaration order, returning the first
/// plaintext ELF that loads. Dies only if every candidate fails; the
/// final message enumerates each candidate with its own typed cause.
pub(crate) fn load_ppu_image_walk_candidates_or_die(
    title: &TitleManifest,
    vfs_root: &Path,
) -> (Vec<u8>, PathBuf) {
    let resolved = title.resolve_eboot(vfs_root).unwrap_or_else(|e| {
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
            match cellgov_firmware::npdrm::decrypt_self_to_elf_auto(&bytes, &resolver) {
                Ok(elf) => return (elf, path),
                Err(e) => {
                    attempts.push((candidate.clone(), LoadCandidateError::Decrypt(e)));
                    continue;
                }
            }
        } else if bytes.len() >= 4 && bytes[..4] == [0x7F, b'E', b'L', b'F'] {
            return (bytes, path);
        } else {
            attempts.push((candidate.clone(), LoadCandidateError::NotElf));
            continue;
        }
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
