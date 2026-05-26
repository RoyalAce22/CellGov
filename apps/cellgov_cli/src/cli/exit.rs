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

/// Why one specific candidate failed to load, kept as a structured
/// value so the candidate-walking loop in
/// [`load_ppu_image_walk_candidates_or_die`] can enumerate every
/// failure with its own typed cause rather than collapsing them.
#[derive(Debug, thiserror::Error)]
enum LoadCandidateError {
    /// `std::fs::read` failed (most often `NotFound`).
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    /// File read but the decrypt path errored with a typed
    /// [`SceError`] (NPDRM klic missing, padding validation
    /// failure, NPDRM-mismatched-key, etc.). The variant is
    /// preserved verbatim -- not collapsed to a generic message --
    /// so the enumerated final error names each candidate's actual
    /// cause via the inner Display chain.
    #[error("{0}")]
    Decrypt(#[from] SceError),
    /// Decrypted bytes were not a valid ELF (loader-side gate; not
    /// expected on a well-formed SELF).
    #[error("bytes are not a SELF or plaintext ELF")]
    NotElf,
}

/// Build the NPDRM klicensee resolver from a title manifest's
/// `rap_filename` field. Reused by every load entry; defined once so
/// the RAP-path layout (`<vfs_root>/home/00000001/exdata/<rap>`) and
/// the typed-error surface stay in one place.
fn klicensee_resolver(
    title: &TitleManifest,
    vfs_root: PathBuf,
) -> impl Fn(&NpdHeaderInfo) -> Option<[u8; 16]> {
    let rap_filename = title.rap_filename.clone();
    move |npd: &NpdHeaderInfo| -> Option<[u8; 16]> {
        // license 3 (free) substitutes NP_KLIC_FREE inside
        // decrypt_self_to_elf_auto if the resolver returns None.
        // license 1/2 require a RAP-backed klicensee here.
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
///
/// Use this for paths the operator named explicitly (e.g. `run-game
/// <path>`); the walking entry
/// [`load_ppu_image_walk_candidates_or_die`] is the right choice
/// when the caller wants the manifest's candidate-list fallthrough.
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

/// Resolve the title's USRDIR per `resolve_eboot`-style rules, then
/// walk `eboot_candidates` in declaration order. For each candidate:
/// open the file; if absent, record `Io::NotFound` and continue to
/// the next; if present, attempt the decrypt; on decrypt success,
/// return the plaintext ELF bytes; on decrypt failure, record the
/// typed [`SceError`] and continue.
///
/// Dies only if every candidate fails. The final message enumerates
/// every candidate with its own typed cause (e.g.
/// `EBOOT.BIN: NoRapForNpdrmTitle(...)`,
/// `EBOOT.elf: read failed: NotFound`), preserving the post-A.4
/// typed-error surface so a missing RAP, a wrong klic, or a
/// genuinely unreadable disk are all distinguishable in the boot
/// failure.
///
/// This wrapper closes the latent bug that turned "CellGov doesn't
/// support NPDRM yet" into "flOw is unbootable": the previous code
/// hard-failed on the first candidate's decrypt error, so a stale
/// or unsupported `EBOOT.BIN` masked an operator-supplied
/// `EBOOT.elf` that would have booted cleanly. For the foundation
/// titles post-A.1..A.5 this fallthrough never fires (the BIN
/// decrypts cleanly via the new NPDRM path) but the resolver is no
/// longer a single point of failure.
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
            // Plaintext ELF: accept directly.
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
