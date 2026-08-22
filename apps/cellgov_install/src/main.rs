//! PS3 firmware and SELF decryption CLI.
//!
//! Exposes [`cellgov_install`]'s library as `install` and
//! `decrypt-self` subcommands.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use cellgov_install::manifest::{
    self, FirmwareFileEntry, FirmwareIdentity, FirmwareManifest, SUPPORTED_FORMAT_VERSION,
};
use cellgov_install::{disc_crypt, game_install, game_uninstall, pup, sce, self_image, tar};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "install" => cmd_install(&args),
        "install-game" => cmd_install_game(&args),
        "install-iso" => cmd_install_iso(&args),
        "uninstall" => cmd_uninstall(&args),
        "decrypt-self" => cmd_decrypt_self(&args),
        _ => {
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  cellgov_install install <PUP_PATH> [--output <dir>] [--force]");
    eprintln!("    default --output: vfs/ (at the current working directory)");
    eprintln!("    extracts the firmware image to dev_flash/ (+ dev_flash2/, dev_flash3/)");
    eprintln!("    --force: overwrite a non-empty dev_flash/, dev_flash2/ or dev_flash3/");
    eprintln!(
        "  cellgov_install install-game <PKG_PATH> [--rap <RAP_PATH>] [--output <dir>] [--force]"
    );
    eprintln!("    default --output: vfs/ (at the current working directory)");
    eprintln!("    --rap: required for license-1/2 NPDRM titles, optional for license-3");
    eprintln!("    --force: overwrite an existing game directory");
    eprintln!(
        "  cellgov_install install-iso <ISO_PATH> [--dkey <DKEY_PATH>] [--output <dir>] [--force]"
    );
    eprintln!("    default --output: vfs/ (at the current working directory)");
    eprintln!("    --dkey: 16-byte disc key for an encrypted image; omit for a decrypted one");
    eprintln!("    extracts the disc tree to dev_bdvd/");
    eprintln!(
        "  cellgov_install uninstall <TITLE_ID> [--output <dir>] [--verify] [--keep-rap] [--force]"
    );
    eprintln!("    default --output: vfs/ (at the current working directory)");
    eprintln!("    --verify: re-hash the live tree against the install record first");
    eprintln!("    --keep-rap: leave the RAP in exdata/ (another title may share it)");
    eprintln!("    --force: uninstall even if --verify finds a modified tree");
    eprintln!("  cellgov_install decrypt-self <SELF_PATH> [--output <path>]");
}

/// Parsed `install` subcommand arguments.
struct InstallArgs {
    pup_path: PathBuf,
    output_dir: PathBuf,
    force: bool,
}

/// `install`'s `--output` names the VFS root, the same root
/// `install-game` and `install-iso` populate.
const DEFAULT_INSTALL_OUTPUT: &str = "vfs";

/// Mount under the VFS root that holds the firmware image and its
/// `firmware.toml` manifest.
const DEV_FLASH_MOUNT: &str = "dev_flash";

/// Why a cellgov_install CLI helper failed.
#[derive(Debug, thiserror::Error)]
enum FirmwareCliError {
    /// `install` invoked without a PUP path.
    #[error("install requires a PUP path")]
    MissingPupPath,
    /// `install-game` invoked without a PKG path.
    #[error("install-game requires a PKG path")]
    MissingPkgPath,
    /// `install-iso` invoked without an ISO path.
    #[error("install-iso requires an ISO path")]
    MissingIsoPath,
    /// `uninstall` invoked without a title-id.
    #[error("uninstall requires a title-id")]
    MissingTitleId,
    /// `decrypt-self` invoked without a SELF path.
    #[error("decrypt-self requires a SELF path")]
    MissingSelfPath,
    /// `--output` flag with no following argument.
    #[error("--output requires a {kind} argument")]
    OutputFlagMissingValue { kind: &'static str },
    /// `--rap` flag with no following argument.
    #[error("--rap requires a path argument")]
    RapFlagMissingValue,
    /// `--dkey` flag with no following argument.
    #[error("--dkey requires a path argument")]
    DkeyFlagMissingValue,
    /// Unknown subcommand flag.
    #[error("unknown argument: {0}")]
    UnknownArgument(String),
    /// `check_output_dir`: read_dir failed on the candidate output.
    #[error("failed to read {}: {source}", path.display())]
    OutputDirReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `check_output_dir`: existing output is non-empty and --force not set.
    #[error("output directory {} exists and is non-empty; pass --force to overwrite", path.display())]
    OutputDirNotEmpty { path: PathBuf },
    /// `collect_sprx_paths`: a directory in the installed firmware tree
    /// could not be listed, so the manifest would silently omit
    /// whatever that subtree holds.
    #[error("read firmware tree {}: {source}", path.display())]
    FirmwareTreeReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `build_firmware_manifest`: reading an SPRX failed.
    #[error("read {}: {source}", path.display())]
    SprxReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `build_firmware_manifest`: `strip_prefix(output_dir)` failed (the
    /// path-walker produced a path that wasn't under `output_dir`, which
    /// implies a `collect_sprx_paths` bug).
    #[error("strip_prefix({}): {source}", path.display())]
    StripPrefixFailed {
        path: PathBuf,
        #[source]
        source: std::path::StripPrefixError,
    },
    /// `build_firmware_manifest`: an SPRX's path has non-UTF-8 bytes;
    /// firmware.toml cannot represent it.
    #[error("non-utf8 firmware path: {}", path.display())]
    NonUtf8Path { path: PathBuf },
}

fn parse_install_args(args: &[String]) -> Result<InstallArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingPupPath);
    }
    let pup_path = PathBuf::from(&args[2]);
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::OutputFlagMissingValue { kind: "directory" });
                }
                output_dir = Some(PathBuf::from(&args[i]));
            }
            "--force" => {
                force = true;
            }
            other => {
                return Err(FirmwareCliError::UnknownArgument(other.to_string()));
            }
        }
        i += 1;
    }
    Ok(InstallArgs {
        pup_path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_INSTALL_OUTPUT)),
        force,
    })
}

/// Errors if `dir` exists and is non-empty without `force`.
fn check_output_dir(dir: &Path, force: bool) -> Result<(), FirmwareCliError> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries =
        std::fs::read_dir(dir).map_err(|source| FirmwareCliError::OutputDirReadFailed {
            path: dir.to_path_buf(),
            source,
        })?;
    if entries.next().is_some() && !force {
        return Err(FirmwareCliError::OutputDirNotEmpty {
            path: dir.to_path_buf(),
        });
    }
    Ok(())
}

/// Parsed `decrypt-self` subcommand arguments.
struct DecryptSelfArgs {
    self_path: PathBuf,
    output_path: Option<PathBuf>,
}

fn parse_decrypt_self_args(args: &[String]) -> Result<DecryptSelfArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingSelfPath);
    }
    let self_path = PathBuf::from(&args[2]);
    let mut output_path: Option<PathBuf> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::OutputFlagMissingValue { kind: "path" });
                }
                output_path = Some(PathBuf::from(&args[i]));
            }
            other => {
                return Err(FirmwareCliError::UnknownArgument(other.to_string()));
            }
        }
        i += 1;
    }
    Ok(DecryptSelfArgs {
        self_path,
        output_path,
    })
}

fn cmd_decrypt_self(args: &[String]) {
    let parsed = parse_decrypt_self_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });
    let self_path = parsed.self_path;
    let output_path = parsed.output_path.unwrap_or_else(|| {
        let stem = self_path.file_stem().unwrap_or_default().to_string_lossy();
        self_path.with_file_name(format!("{stem}.elf"))
    });

    let data = std::fs::read(&self_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", self_path.display());
        std::process::exit(1);
    });
    println!(
        "cellgov_install: decrypting {} ({:.1} MB)",
        self_path.display(),
        data.len() as f64 / (1024.0 * 1024.0)
    );

    let elf = sce::decrypt_self_to_elf(&data).unwrap_or_else(|e| {
        eprintln!("SELF decryption failed: {e}");
        std::process::exit(1);
    });

    std::fs::write(&output_path, &elf).unwrap_or_else(|e| {
        eprintln!("failed to write {}: {e}", output_path.display());
        std::process::exit(1);
    });
    println!(
        "cellgov_install: wrote {} ({:.1} MB)",
        output_path.display(),
        elf.len() as f64 / (1024.0 * 1024.0)
    );
}

/// dev_flash subtrees CellGov never loads and prunes at install time:
/// the PS1 / PS2 / PSP backward-compat emulators, which a CBE
/// execution oracle never runs.
const PRUNED_DEV_FLASH_DIRS: [&str; 3] = ["ps1emu/", "ps2emu/", "pspemu/"];

/// Whether an inner dev_flash entry is dropped at install time.
///
/// Two reasons: RPCS3's fullwidth-dollar (`U+FF04`) backwards-compat
/// dead-entry marker (never written to disk, matching
/// `tar_object::extract`), and CellGov's emulator prune
/// ([`PRUNED_DEV_FLASH_DIRS`]).
///
/// The prune decides on [`tar::route_entry_path`]'s output so it sees
/// the exact path the extractor would write, whatever the `000/`
/// packaging or leading slash the raw name carries.
fn is_install_excluded(entry_name: &str) -> bool {
    if entry_name.contains('\u{ff04}') {
        return true;
    }
    let Some(routed) = tar::route_entry_path(entry_name) else {
        return false;
    };
    let Some(rel) = routed.strip_prefix("dev_flash/") else {
        return false;
    };
    PRUNED_DEV_FLASH_DIRS.iter().any(|d| rel.starts_with(d))
}

/// Mounts under the VFS root that `install` writes into: the firmware
/// image plus its siblings. `firmware.toml` covers
/// [`DEV_FLASH_MOUNT`] alone.
fn firmware_mounts() -> impl Iterator<Item = &'static str> {
    std::iter::once(DEV_FLASH_MOUNT)
        .chain(tar::SIBLING_MOUNTS.iter().map(|m| m.trim_end_matches('/')))
}

/// Refuse the install when any mount it writes is already populated.
///
/// Scoped to [`firmware_mounts`] rather than the whole VFS root, which
/// legitimately already holds `dev_hdd0` / `dev_bdvd` from a game
/// install.
///
/// # Errors
///
/// [`FirmwareCliError::OutputDirNotEmpty`] naming the first occupied
/// mount, or [`FirmwareCliError::OutputDirReadFailed`].
fn preflight_firmware_mounts(output_dir: &Path, force: bool) -> Result<(), FirmwareCliError> {
    for mount in firmware_mounts() {
        check_output_dir(&output_dir.join(mount), force)?;
    }
    Ok(())
}

fn cmd_install(args: &[String]) {
    let install_args = parse_install_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });
    let InstallArgs {
        pup_path,
        output_dir,
        force,
    } = install_args;

    let dev_flash_dir = output_dir.join(DEV_FLASH_MOUNT);
    preflight_firmware_mounts(&output_dir, force).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let pup_data = std::fs::read(&pup_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", pup_path.display());
        std::process::exit(1);
    });

    println!(
        "cellgov_install: reading {} ({:.1} MB)",
        pup_path.display(),
        pup_data.len() as f64 / (1024.0 * 1024.0)
    );

    let pup = pup::parse(&pup_data).unwrap_or_else(|e| {
        eprintln!("PUP parse error: {e}");
        std::process::exit(1);
    });
    println!(
        "  PUP version: {}, {} entries",
        pup.image_version,
        pup.entries.len()
    );

    println!("  validating HMAC...");
    pup::validate_hashes(&pup_data, &pup).unwrap_or_else(|e| {
        eprintln!("PUP hash validation failed: {e}");
        std::process::exit(1);
    });
    println!("  all entries valid");

    let update_entry = pup
        .entries
        .iter()
        .find(|e| e.entry_id == 0x300)
        .unwrap_or_else(|| {
            eprintln!("PUP has no entry 0x300 (update_files)");
            std::process::exit(1);
        });

    let update_data =
        &pup_data[update_entry.data_offset as usize..][..update_entry.data_length as usize];
    let outer_tar = tar::parse(update_data).unwrap_or_else(|e| {
        eprintln!("PUP outer TAR parse failed: {e}");
        std::process::exit(1);
    });
    // Match RPCS3: only packages whose name contains `dev_flash_`
    // (with the trailing underscore) are firmware payload. This drops
    // the `dev_flash3_*` revocation-list package RPCS3 also skips.
    let dev_flash_entries: Vec<_> = outer_tar
        .iter()
        .filter(|e| e.name.contains("dev_flash_"))
        .collect();

    println!(
        "  update_files TAR: {} entries, {} dev_flash packages",
        outer_tar.len(),
        dev_flash_entries.len()
    );

    let mut total_files = 0usize;
    let mut total_skipped = 0usize;
    let mut total_pruned = 0usize;
    let mut packages_attempted = 0usize;
    let mut extract_errors: Vec<tar::ExtractError> = Vec::new();
    for entry in &dev_flash_entries {
        packages_attempted += 1;
        let short = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        print!("  decrypting {short}...");
        match sce::decrypt_package(&entry.data) {
            Ok(inner_tar_data) => match tar::parse(&inner_tar_data) {
                Ok(inner_files) => {
                    let packaged = inner_files.len();
                    let inner_files: Vec<tar::TarEntry> = inner_files
                        .into_iter()
                        .filter(|f| !is_install_excluded(&f.name))
                        .collect();
                    let pruned = packaged - inner_files.len();
                    total_pruned += pruned;
                    if inner_files.is_empty() {
                        println!(" empty ({packaged} entries, {pruned} pruned)");
                        continue;
                    }
                    let report = tar::extract_to_disk(&inner_files, &output_dir);
                    total_files += report.written;
                    total_skipped += report.skipped;
                    print!(" {} files", report.written);
                    if pruned > 0 {
                        print!(", {pruned} pruned");
                    }
                    if report.skipped > 0 {
                        print!(", {} entries addressing no file", report.skipped);
                    }
                    if !report.errors.is_empty() {
                        print!(", {} extract errors", report.errors.len());
                    }
                    println!();
                    extract_errors.extend(report.errors);
                }
                Err(e) => {
                    println!(" skip (inner TAR parse: {e})");
                }
            },
            Err(e) => {
                println!(" skip ({e})");
            }
        }
    }

    if !extract_errors.is_empty() {
        eprintln!("cellgov_install: {} extract errors:", extract_errors.len());
        for err in &extract_errors {
            eprintln!("  {err}");
        }
    }

    if total_files == 0 {
        eprintln!(
            "cellgov_install: install produced 0 files (attempted {packages_attempted} dev_flash packages); refusing to claim success"
        );
        std::process::exit(1);
    }

    println!(
        "cellgov_install: installed {} files to {} ({} packages, {} pruned, {} skipped, {} errors)",
        total_files,
        output_dir.display(),
        packages_attempted,
        total_pruned,
        total_skipped,
        extract_errors.len(),
    );

    print!("  building firmware.toml...");
    // Rooted at the dev_flash mount so entry paths stay
    // `sys/external/...` and `cellgov_cli`'s walk-up from the firmware
    // dir finds the manifest beside the tree it covers.
    let manifest = match build_firmware_manifest(&pup_data, pup.image_version, &dev_flash_dir) {
        Ok(m) => m,
        Err(e) => {
            println!(" FAILED ({e})");
            std::process::exit(1);
        }
    };
    let manifest_path = dev_flash_dir.join("firmware.toml");
    let text = manifest::serialize_manifest(&manifest).unwrap_or_else(|e| {
        eprintln!("\nfirmware.toml serialise failed: {e}");
        std::process::exit(1);
    });
    std::fs::write(&manifest_path, text).unwrap_or_else(|e| {
        eprintln!(
            "\nfirmware.toml write to {} failed: {e}",
            manifest_path.display()
        );
        std::process::exit(1);
    });
    println!(
        " wrote {} ({} entries)",
        manifest_path.display(),
        manifest.files.len()
    );
}

/// Parsed `install-game` subcommand arguments.
struct InstallGameArgs {
    pkg_path: PathBuf,
    rap_path: Option<PathBuf>,
    output_dir: PathBuf,
    force: bool,
}

const DEFAULT_GAME_INSTALL_OUTPUT: &str = "vfs";
/// Install records live at the project root, outside the PS3-shaped
/// `vfs/` tree.
const INSTALLS_DIR: &str = "installs";

fn parse_install_game_args(args: &[String]) -> Result<InstallGameArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingPkgPath);
    }
    let pkg_path = PathBuf::from(&args[2]);
    let mut rap_path: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--rap" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::RapFlagMissingValue);
                }
                rap_path = Some(PathBuf::from(&args[i]));
            }
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::OutputFlagMissingValue { kind: "directory" });
                }
                output_dir = Some(PathBuf::from(&args[i]));
            }
            "--force" => force = true,
            other => return Err(FirmwareCliError::UnknownArgument(other.to_string())),
        }
        i += 1;
    }
    Ok(InstallGameArgs {
        pkg_path,
        rap_path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        force,
    })
}

fn cmd_install_game(args: &[String]) {
    let parsed = parse_install_game_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    let pkg_data = std::fs::read(&parsed.pkg_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", parsed.pkg_path.display());
        std::process::exit(1);
    });
    let rap_data = parsed.rap_path.as_ref().map(|p| {
        std::fs::read(p).unwrap_or_else(|e| {
            eprintln!("failed to read RAP {}: {e}", p.display());
            std::process::exit(1);
        })
    });

    println!(
        "cellgov_install: installing game from {} ({:.1} MB)",
        parsed.pkg_path.display(),
        pkg_data.len() as f64 / (1024.0 * 1024.0)
    );

    let installs_dir = PathBuf::from(INSTALLS_DIR);
    let outcome = game_install::install_pkg(
        &pkg_data,
        rap_data.as_deref(),
        &parsed.output_dir,
        &installs_dir,
        parsed.force,
    )
    .unwrap_or_else(|e| {
        eprintln!("install-game failed: {e}");
        std::process::exit(1);
    });

    println!(
        "  title {} (content {}): {} files -> {}",
        outcome.title_id,
        outcome.content_id,
        outcome.file_count,
        outcome.game_dir.display(),
    );
    println!(
        "  RAP {}, record {}",
        if outcome.rap_installed {
            "installed"
        } else {
            "not installed (none required)"
        },
        outcome.record_path.display(),
    );
}

/// Parsed `install-iso` subcommand arguments.
struct InstallIsoArgs {
    iso_path: PathBuf,
    dkey_path: Option<PathBuf>,
    output_dir: PathBuf,
    force: bool,
}

fn parse_install_iso_args(args: &[String]) -> Result<InstallIsoArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingIsoPath);
    }
    let iso_path = PathBuf::from(&args[2]);
    let mut dkey_path: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--dkey" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::DkeyFlagMissingValue);
                }
                dkey_path = Some(PathBuf::from(&args[i]));
            }
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::OutputFlagMissingValue { kind: "directory" });
                }
                output_dir = Some(PathBuf::from(&args[i]));
            }
            "--force" => force = true,
            other => return Err(FirmwareCliError::UnknownArgument(other.to_string())),
        }
        i += 1;
    }
    Ok(InstallIsoArgs {
        iso_path,
        dkey_path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        force,
    })
}

fn cmd_install_iso(args: &[String]) {
    let parsed = parse_install_iso_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    let iso_data = std::fs::read(&parsed.iso_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", parsed.iso_path.display());
        std::process::exit(1);
    });

    println!(
        "cellgov_install: installing disc from {} ({:.1} MB)",
        parsed.iso_path.display(),
        iso_data.len() as f64 / (1024.0 * 1024.0)
    );

    // With --dkey the image is encrypted: decrypt it first, then the
    // record's source hash is over the original (encrypted) bytes. With
    // no --dkey the image is assumed already decrypted.
    let decrypted = match &parsed.dkey_path {
        Some(dkey_path) => {
            let dkey = std::fs::read(dkey_path).unwrap_or_else(|e| {
                eprintln!("failed to read disc key {}: {e}", dkey_path.display());
                std::process::exit(1);
            });
            let dkey: [u8; 16] = dkey.as_slice().try_into().unwrap_or_else(|_| {
                eprintln!("disc key must be exactly 16 bytes, got {}", dkey.len());
                std::process::exit(1);
            });
            println!("  decrypting protected sectors with supplied disc key...");
            disc_crypt::decrypt_disc_image(&iso_data, &dkey).unwrap_or_else(|e| {
                eprintln!("disc decryption failed: {e}");
                std::process::exit(1);
            })
        }
        None => iso_data.clone(),
    };

    let installs_dir = PathBuf::from(INSTALLS_DIR);
    let outcome = game_install::install_iso(
        &decrypted,
        &iso_data,
        &parsed.output_dir,
        &installs_dir,
        parsed.force,
    )
    .unwrap_or_else(|e| {
        eprintln!("install-iso failed: {e}");
        std::process::exit(1);
    });

    println!(
        "  title {}: {} files -> {}",
        outcome.title_id,
        outcome.file_count,
        outcome.game_dir.display(),
    );
    println!("  record {}", outcome.record_path.display());
}

/// Parsed `uninstall` subcommand arguments.
struct UninstallArgs {
    title_id: String,
    output_dir: PathBuf,
    verify: bool,
    keep_rap: bool,
    force: bool,
}

fn parse_uninstall_args(args: &[String]) -> Result<UninstallArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingTitleId);
    }
    let title_id = args[2].clone();
    let mut output_dir: Option<PathBuf> = None;
    let mut verify = false;
    let mut keep_rap = false;
    let mut force = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::OutputFlagMissingValue { kind: "directory" });
                }
                output_dir = Some(PathBuf::from(&args[i]));
            }
            "--verify" => verify = true,
            "--keep-rap" => keep_rap = true,
            "--force" => force = true,
            other => return Err(FirmwareCliError::UnknownArgument(other.to_string())),
        }
        i += 1;
    }
    Ok(UninstallArgs {
        title_id,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        verify,
        keep_rap,
        force,
    })
}

fn cmd_uninstall(args: &[String]) {
    let parsed = parse_uninstall_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    let installs_dir = PathBuf::from(INSTALLS_DIR);
    let opts = game_uninstall::UninstallOptions {
        verify: parsed.verify,
        keep_rap: parsed.keep_rap,
        force: parsed.force,
    };
    let outcome =
        game_uninstall::uninstall(&parsed.title_id, &parsed.output_dir, &installs_dir, opts)
            .unwrap_or_else(|e| {
                eprintln!("uninstall failed: {e}");
                std::process::exit(1);
            });

    println!("cellgov_install: uninstalled {}", outcome.title_id);
    println!("  removed game dir {}", outcome.game_dir_removed.display());
    if let Some(rap) = &outcome.rap_removed {
        println!("  removed RAP {}", rap.display());
    }
    if let Some(n) = outcome.files_verified {
        println!("  verified {n} files against the record before removal");
    }
    println!("  removed record {}", outcome.record_removed.display());
}

/// Walk `dir` recursively in lexicographic order and append every
/// path ending in `.sprx` or `.prx` to `paths`. `.prx` covers
/// pre-decrypted corpora, which the boot verifier loads through the
/// same manifest path.
///
/// # Errors
///
/// [`FirmwareCliError::FirmwareTreeReadFailed`] for any directory or
/// directory entry the walk cannot read; the walk aborts rather than
/// emitting a manifest short of the modules it claims to cover.
fn collect_sprx_paths(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<(), FirmwareCliError> {
    let entries =
        std::fs::read_dir(dir).map_err(|source| FirmwareCliError::FirmwareTreeReadFailed {
            path: dir.to_path_buf(),
            source,
        })?;
    let mut sorted: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| FirmwareCliError::FirmwareTreeReadFailed {
            path: dir.to_path_buf(),
            source,
        })?;
        sorted.push(entry.path());
    }
    sorted.sort();
    for p in sorted {
        if p.is_dir() {
            collect_sprx_paths(&p, paths)?;
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx") || x.eq_ignore_ascii_case("prx"))
        {
            paths.push(p);
        }
    }
    Ok(())
}

/// Build the firmware.toml manifest from a freshly-installed tree.
/// PUP hash is over `pup_data`; per-file hashes are over the post-
/// decrypt ELF bytes. Files that fail to decrypt (e.g. a revision with
/// no APP key) are left out of the manifest and their count is
/// reported on stderr.
///
/// # Errors
///
/// [`FirmwareCliError::FirmwareTreeReadFailed`] when the walk cannot
/// list part of the tree, plus the per-file read / strip_prefix /
/// non-UTF-8 refusals.
fn build_firmware_manifest(
    pup_data: &[u8],
    pup_image_version: u64,
    output_dir: &Path,
) -> Result<FirmwareManifest, FirmwareCliError> {
    let mut pup_hasher = Sha256::new();
    pup_hasher.update(pup_data);
    let pup_sha256 = manifest::Sha256(pup_hasher.finalize().into());

    let mut sprx_paths = Vec::new();
    collect_sprx_paths(output_dir, &mut sprx_paths)?;

    let mut files = Vec::with_capacity(sprx_paths.len());
    let mut skipped = 0usize;
    for sprx_path in &sprx_paths {
        let raw = std::fs::read(sprx_path).map_err(|source| FirmwareCliError::SprxReadFailed {
            path: sprx_path.clone(),
            source,
        })?;
        // Pre-decrypted `.prx` files carry no SCE wrapper: hash the
        // raw bytes (identical to their post-decrypt image) and record
        // revision 0, since the wrapper that carried it is gone.
        let (elf, revision) = if self_image::is_sce_wrapped(&raw) {
            let elf = match sce::decrypt_self_to_elf(&raw) {
                Ok(e) => e,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            // decrypt_self_to_elf already parsed the same header to get
            // here, so this parse cannot fail.
            let revision = sce::parse_sce_header(&raw)
                .expect("decrypt_self_to_elf success implies parse_sce_header success")
                .revision_flags
                & 0x7FFF;
            (elf, revision)
        } else {
            (raw, 0)
        };
        let mut h = Sha256::new();
        h.update(&elf);
        let sha256 = manifest::Sha256(h.finalize().into());
        let rel = sprx_path.strip_prefix(output_dir).map_err(|source| {
            FirmwareCliError::StripPrefixFailed {
                path: sprx_path.clone(),
                source,
            }
        })?;
        let path = rel
            .to_str()
            .ok_or_else(|| FirmwareCliError::NonUtf8Path {
                path: rel.to_path_buf(),
            })?
            .replace('\\', "/");
        files.push(FirmwareFileEntry {
            path,
            sha256,
            revision,
        });
    }
    if skipped > 0 {
        eprintln!("  ({skipped} SPRX skipped: undecryptable)");
    }

    Ok(FirmwareManifest {
        format_version: SUPPORTED_FORMAT_VERSION,
        firmware: FirmwareIdentity {
            // PUP-header `image_version` is an opaque u64 identifier
            // (RPCS3 reads the user-facing version from
            // `vsh/etc/version.txt` instead). Rendered as zero-padded
            // hex so firmware.toml shows the raw value.
            image_version: format!("0x{pup_image_version:016x}"),
            pup_sha256,
        },
        files,
    })
}

#[cfg(test)]
mod scratch_dir;

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;
