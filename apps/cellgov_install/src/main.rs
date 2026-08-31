//! PS3 firmware and SELF decryption CLI.
//!
//! Exposes [`cellgov_install`]'s library as `install` and
//! `decrypt-self` subcommands. Every subcommand but `uninstall` and
//! `keys` decrypts, so a binary built without the `decrypt` feature
//! refuses them by name. The decrypting ones read their key material
//! from the operator's vault (`CELLGOV_KEYS`, or the one `keys import`
//! normalized under the VFS root).

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the argument parsers are reachable only from the gated subcommands; the feature-on build lints them"
    )
)]

use cellgov_ps3_abi::elf::ELF_MAGIC;

use cellgov_install::game_install::InstallOptions;
use cellgov_install::keys::{
    installed_keys_dir, KeyVault, KeyVaultError, SelfClass, Slot, ENV_KEYS, INSTALLED_KEYS_FILE,
};
use cellgov_install::manifest::{
    self, FirmwareFileEntry, FirmwareIdentity, FirmwareManifest, SUPPORTED_FORMAT_VERSION,
};
use cellgov_install::npdrm::{NpdHeaderInfo, Rap};
use cellgov_install::progress::INSTALL_TASK;
use cellgov_install::{game_install, game_uninstall, pup, sce, self_image, tar};
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        #[cfg(feature = "decrypt")]
        "install" => cmd_install(&args),
        #[cfg(feature = "decrypt")]
        "install-game" => cmd_install_game(&args),
        #[cfg(feature = "decrypt")]
        "install-iso" => cmd_install_iso(&args),
        "uninstall" => cmd_uninstall(&args),
        "keys" => cmd_keys(&args),
        #[cfg(feature = "decrypt")]
        "decrypt-self" => cmd_decrypt_self(&args),
        #[cfg(not(feature = "decrypt"))]
        sub @ ("install" | "install-game" | "install-iso" | "decrypt-self") => {
            eprintln!(
                "{}",
                FirmwareCliError::DecryptFeatureDisabled {
                    subcommand: sub.to_string(),
                }
            );
            std::process::exit(1);
        }
        _ => {
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    if cfg!(feature = "decrypt") {
        eprintln!(
            "  every decrypting subcommand reads the operator's key vault: {ENV_KEYS}=<file-or-dir>,"
        );
        eprintln!("    or the vault `keys import` wrote under <vfs>/.cellgov/keys/");
        eprintln!("  cellgov_install install <PUP_PATH> [--output <dir>] [--force]");
        eprintln!("    default --output: vfs/ (at the current working directory)");
        eprintln!("    extracts the firmware image to dev_flash/ (+ dev_flash2/, dev_flash3/)");
        eprintln!("    --force: overwrite a non-empty dev_flash/, dev_flash2/ or dev_flash3/");
        eprintln!(
            "  cellgov_install install-game <PKG_PATH> [--rap <RAP_PATH>] [--output <dir>] [--force]"
        );
        eprintln!("    [--no-progress] [--no-color] [--quiet]: progress-bar overrides");
        eprintln!("    default --output: vfs/ (at the current working directory)");
        eprintln!("    --rap: required for license-1/2 NPDRM titles, optional for license-3");
        eprintln!("    --force: overwrite an existing game directory");
        eprintln!("  cellgov_install install-iso <ISO_PATH> [--output <dir>] [--force]");
        eprintln!("    [--no-progress] [--no-color] [--quiet]: progress-bar overrides");
        eprintln!("    default --output: vfs/ (at the current working directory)");
        eprintln!("    takes a decrypted dump of a disc you own; an encrypted image is refused");
        eprintln!("    extracts the disc tree to dev_bdvd/");
    } else {
        eprintln!("  this build has no decrypt support: only `uninstall` and `keys` run.");
        eprintln!(
            "    `install`, `install-game`, `install-iso` and `decrypt-self` are refused by name;"
        );
        eprintln!("    rebuild with `--features decrypt` to get them.");
    }
    eprintln!(
        "  cellgov_install uninstall <TITLE_ID> [--output <dir>] [--verify] [--keep-rap] [--force]"
    );
    eprintln!("    default --output: vfs/ (at the current working directory)");
    eprintln!("    --verify: re-hash the live tree against the install record first");
    eprintln!("    --keep-rap: leave the RAP in exdata/ (another title may share it)");
    eprintln!("    --force: uninstall even if --verify finds a modified tree");
    if cfg!(feature = "decrypt") {
        eprintln!(
            "  cellgov_install decrypt-self <SELF_PATH> [--output <path>] [--rap <RAP_PATH>] [--vfs-root <dir>]"
        );
        eprintln!(
            "    NPDRM SELFs resolve their RAP from <vfs-root>/dev_hdd0/home/00000001/exdata/"
        );
        eprintln!("    --rap: use this RAP instead, for a title that is not installed");
        eprintln!("    default --vfs-root: vfs/ (at the current working directory)");
    }
    eprintln!("  cellgov_install keys show [PATH] [--output <vfs>]");
    eprintln!("    inventory of the vault at PATH, else of {ENV_KEYS} / the imported one");
    eprintln!("    exits 2 when a decrypt path would find a key missing");
    eprintln!("  cellgov_install keys import <PATH> [--output <vfs>] [--replace]");
    eprintln!("    normalizes a keys file or directory into <vfs>/.cellgov/keys/keys.toml");
    eprintln!("    --replace: drop the imported vault already there instead of merging into it");
    eprintln!("  cellgov_install keys remove [--output <vfs>]");
    eprintln!("    deletes <vfs>/.cellgov/keys/");
    eprintln!("    default --output: vfs/ (at the current working directory)");
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
    /// `keys` invoked without `show`, `import`, or `remove`.
    #[error("keys requires a subcommand: show, import, or remove")]
    KeysMissingSubcommand,
    /// `keys import` invoked without a keys file or directory.
    #[error("keys import requires the path of a keys file or directory")]
    KeysImportMissingPath,
    /// `keys` invoked with a subcommand it does not have.
    #[error("unknown keys subcommand: {0} (expected show, import, or remove)")]
    KeysUnknownSubcommand(String),
    /// A keys subcommand given a second positional argument.
    #[error("keys {subcommand} takes at most one path; unexpected argument: {extra}")]
    KeysExtraPositional {
        subcommand: &'static str,
        extra: String,
    },
    /// Loading, merging, or locating a vault failed.
    #[error("{0}")]
    Keys(#[from] KeyVaultError),
    /// `keys import`: the imported file or directory held no key the
    /// decrypt paths could use.
    #[error("keys import: {} holds no scalar key, SCE package keyset, or APP/NPDRM keyset; nothing to import (run `keys show {}` to see what was read and set aside)", path.display(), path.display())]
    KeysNothingUsable { path: PathBuf },
    /// `keys import`: the installed-vault directory could not be created.
    #[error("create {}: {source}", path.display())]
    KeysDirCreateFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `keys import`: `keys.toml` could not be written.
    #[error("write {}: {source}", path.display())]
    KeysWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `keys remove`: the installed-vault directory exists and could
    /// not be deleted.
    #[error("remove {}: {source}", path.display())]
    KeysRemoveFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A decrypting subcommand on a binary built without the
    /// `decrypt` cargo feature.
    #[cfg(not(feature = "decrypt"))]
    #[error("{subcommand} decrypts, and this cellgov_install was built without the `decrypt` cargo feature; rebuild with `cargo build -p cellgov_install --features decrypt`")]
    DecryptFeatureDisabled { subcommand: String },
    /// `--output` flag with no following argument.
    #[error("--output requires a {kind} argument")]
    OutputFlagMissingValue { kind: &'static str },
    /// `--rap` flag with no following argument.
    #[error("--rap requires a path argument")]
    RapFlagMissingValue,
    /// `--vfs-root` flag with no following argument.
    #[error("--vfs-root requires a directory argument")]
    VfsRootFlagMissingValue,
    /// A RAP file is not the 16 bytes the klicensee derivation needs.
    #[error("RAP {} is {len} bytes; expected exactly 16", path.display())]
    RapWrongSize { path: PathBuf, len: usize },
    /// A RAP file exists but could not be read.
    #[error("read RAP {}: {source}", path.display())]
    RapReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// `--rap` named a file that is not there. Distinct from the
    /// exdata probe, whose miss is the ordinary uninstalled case.
    #[error("--rap {} does not exist", path.display())]
    ExplicitRapMissing { path: PathBuf },
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
    rap_path: Option<PathBuf>,
    vfs_root: PathBuf,
}

fn parse_decrypt_self_args(args: &[String]) -> Result<DecryptSelfArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingSelfPath);
    }
    let self_path = PathBuf::from(&args[2]);
    let mut output_path: Option<PathBuf> = None;
    let mut rap_path: Option<PathBuf> = None;
    let mut vfs_root: Option<PathBuf> = None;
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
            "--rap" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::RapFlagMissingValue);
                }
                rap_path = Some(PathBuf::from(&args[i]));
            }
            "--vfs-root" => {
                i += 1;
                if i >= args.len() {
                    return Err(FirmwareCliError::VfsRootFlagMissingValue);
                }
                vfs_root = Some(PathBuf::from(&args[i]));
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
        rap_path,
        vfs_root: vfs_root.unwrap_or_else(|| PathBuf::from(DEFAULT_INSTALL_OUTPUT)),
    })
}

/// Read a 16-byte RAP file.
///
/// # Errors
///
/// [`FirmwareCliError::RapWrongSize`] for a file that is not exactly
/// 16 bytes, and [`FirmwareCliError::RapReadFailed`] for a file that is
/// there but unreadable. Only absence is `Ok(None)`: that is the
/// ordinary "not installed" case, which the NPDRM layer turns into
/// either the license-3 free-key fallback or a named refusal. Any other
/// read failure would otherwise be indistinguishable from absence and
/// slip through as the free key.
fn rap_from_file(path: &Path) -> Result<Option<Rap>, FirmwareCliError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(FirmwareCliError::RapReadFailed {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let arr: [u8; 16] =
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| FirmwareCliError::RapWrongSize {
                path: path.to_path_buf(),
                len: bytes.len(),
            })?;
    Ok(Some(Rap(arr)))
}

/// Resolve the RAP for one NPDRM content id: from `explicit` when
/// `--rap` named a file, otherwise from `<exdata>/<id>.rap`.
///
/// # Errors
///
/// Everything [`rap_from_file`] refuses, plus
/// [`FirmwareCliError::ExplicitRapMissing`] when `--rap` named a file
/// that is not there. Only the exdata probe may miss quietly -- an
/// explicit flag that resolved to nothing would otherwise be indis-
/// tinguishable from not passing it, and license-3 titles decrypt on
/// the free-key fallback either way.
fn resolve_rap(
    explicit: Option<&Path>,
    exdata: &Path,
    content_id: &str,
) -> Result<Option<Rap>, FirmwareCliError> {
    let rap = match explicit {
        Some(p) => p.to_path_buf(),
        None => exdata.join(format!("{content_id}.rap")),
    };
    match rap_from_file(&rap)? {
        Some(k) => Ok(Some(k)),
        None if explicit.is_some() => Err(FirmwareCliError::ExplicitRapMissing { path: rap }),
        None => Ok(None),
    }
}

#[cfg(feature = "decrypt")]
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

    let keys = KeyVault::load_for_vfs(&parsed.vfs_root).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
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

    // Auto, not AppOnly: firmware and disc SELFs are APP-keyed and
    // take the same path they always did, while an NPDRM title finds
    // its klicensee the way the boot path does.
    let exdata = cellgov_install::store::StoreLayout::new(&parsed.vfs_root).live_exdata_dir();
    let resolve_error: RefCell<Option<FirmwareCliError>> = RefCell::new(None);
    let resolver = |npd: &NpdHeaderInfo| -> Option<Rap> {
        match resolve_rap(parsed.rap_path.as_deref(), &exdata, &npd.content_id) {
            Ok(k) => k,
            Err(e) => {
                // A refused RAP is a hard error, but the resolver
                // signature can only say "no key". Carry it out so the
                // exit names the file instead of the missing key.
                *resolve_error.borrow_mut() = Some(e);
                None
            }
        }
    };

    let decrypted =
        self_image::to_plaintext_elf(&data, &keys, self_image::KeyPolicy::Auto(&resolver));

    // Checked whichever way the decrypt went. A license-3 SELF falls
    // back to the vault's free klicensee when the resolver yields no key, so a
    // refused RAP would otherwise be swallowed by a "successful"
    // free-key decrypt that ignored the RAP the caller supplied.
    if let Some(rap_err) = resolve_error.borrow_mut().take() {
        eprintln!("{rap_err}");
        std::process::exit(1);
    }

    let elf = decrypted.unwrap_or_else(|e| {
        eprintln!("SELF decryption failed: {e}");
        // Only reachable without --rap: an explicit RAP that will not
        // read is already refused by name above.
        if let sce::SceError::NoRapForNpdrmTitle { .. } = e {
            eprintln!(
                "  searched {} for <content_id>.rap; pass --rap <path> for an uninstalled title",
                exdata.display()
            );
        }
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

#[cfg(feature = "decrypt")]
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
    let keys = KeyVault::load_for_vfs(&output_dir).unwrap_or_else(|e| {
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
    pup::validate_hashes(&pup_data, &pup, &keys).unwrap_or_else(|e| {
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
    let mut packages_failed = 0usize;
    let mut extract_errors: Vec<tar::ExtractError> = Vec::new();
    for entry in &dev_flash_entries {
        packages_attempted += 1;
        let short = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        print!("  decrypting {short}...");
        match sce::decrypt_package(&entry.data, &keys) {
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
                    packages_failed += 1;
                    println!(" FAILED (inner TAR parse: {e})");
                }
            },
            Err(e) => {
                packages_failed += 1;
                println!(" FAILED ({e})");
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

    // A dropped package or a failed write leaves the tree short of the
    // firmware the PUP carries, and firmware.toml built over it would
    // record the gap as if it were the image. RPCS3 refuses the same
    // way: `main_window.cpp` `HandlePupInstallation` aborts the whole
    // install when a dev_flash sub-package will not decrypt or when
    // `Loader/TAR.cpp` `tar_object::extract` reports a failed write,
    // and announces success only once every package landed.
    if packages_failed > 0 || !extract_errors.is_empty() {
        eprintln!(
            "cellgov_install: partial install ({total_files} files, {packages_failed} of \
             {packages_attempted} packages failed, {} extract errors); refusing to claim success",
            extract_errors.len(),
        );
        std::process::exit(1);
    }

    println!(
        "cellgov_install: installed {} files to {} ({} packages, {} pruned, {} skipped, {} failed packages, {} errors)",
        total_files,
        output_dir.display(),
        packages_attempted,
        total_pruned,
        total_skipped,
        packages_failed,
        extract_errors.len(),
    );

    print!("  building firmware.toml...");
    // Rooted at the dev_flash mount so entry paths stay
    // `sys/external/...` and `cellgov_cli`'s walk-up from the firmware
    // dir finds the manifest beside the tree it covers.
    let manifest =
        match build_firmware_manifest(&pup_data, pup.image_version, &dev_flash_dir, &keys) {
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
    render: RenderFlags,
}

const DEFAULT_GAME_INSTALL_OUTPUT: &str = cellgov_install::store::DEFAULT_VFS_ROOT;

fn parse_install_game_args(args: &[String]) -> Result<InstallGameArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingPkgPath);
    }
    let pkg_path = PathBuf::from(&args[2]);
    let mut rap_path: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut render = RenderFlags::default();
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
            other if render.accept(other) => {}
            other => return Err(FirmwareCliError::UnknownArgument(other.to_string())),
        }
        i += 1;
    }
    Ok(InstallGameArgs {
        pkg_path,
        rap_path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        force,
        render,
    })
}

/// The container's filename, for the progress bar's title line.
fn container_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(feature = "decrypt")]
fn cmd_install_game(args: &[String]) {
    let parsed = parse_install_game_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    // Map the package instead of reading it, as install-iso does: the
    // install reads it sequentially (header, one CTR pass, the source
    // hash), so pages stream in and are evicted rather than committed.
    let pkg_data = filebuffer::FileBuffer::open(&parsed.pkg_path).unwrap_or_else(|e| {
        eprintln!("failed to map {}: {e}", parsed.pkg_path.display());
        std::process::exit(1);
    });
    let rap_data = parsed.rap_path.as_ref().map(|p| {
        std::fs::read(p).unwrap_or_else(|e| {
            eprintln!("failed to read RAP {}: {e}", p.display());
            std::process::exit(1);
        })
    });
    let keys = KeyVault::load_for_vfs(&parsed.output_dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    println!(
        "cellgov_install: installing game from {} ({:.1} MB)",
        parsed.pkg_path.display(),
        pkg_data.len() as f64 / (1024.0 * 1024.0)
    );

    let bar = ProgressBar::start(
        parsed.render.caps(),
        &INSTALL_TASK,
        &container_label(&parsed.pkg_path),
    );
    let reporter = bar.sink();
    let outcome = game_install::install_pkg(
        &pkg_data,
        rap_data.as_deref(),
        &keys,
        &parsed.output_dir,
        InstallOptions {
            force: parsed.force,
            progress: &*reporter,
        },
    );
    let outcome = match outcome {
        Ok(o) => {
            bar.finish();
            o
        }
        Err(e) => {
            bar.abort();
            eprintln!("install-game failed: {e}");
            std::process::exit(1);
        }
    };

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
    output_dir: PathBuf,
    force: bool,
    render: RenderFlags,
}

fn parse_install_iso_args(args: &[String]) -> Result<InstallIsoArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingIsoPath);
    }
    let iso_path = PathBuf::from(&args[2]);
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut render = RenderFlags::default();
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
            "--force" => force = true,
            other if render.accept(other) => {}
            other => return Err(FirmwareCliError::UnknownArgument(other.to_string())),
        }
        i += 1;
    }
    Ok(InstallIsoArgs {
        iso_path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        force,
        render,
    })
}

#[cfg(feature = "decrypt")]
fn cmd_install_iso(args: &[String]) {
    let parsed = parse_install_iso_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    // Map the image instead of reading it: a disc image can exceed
    // host RAM, and the install only ever reads it (twice, both
    // sequentially -- the carve and the source hash), so pages stream
    // in and are evicted rather than committed all at once.
    let iso_data = filebuffer::FileBuffer::open(&parsed.iso_path).unwrap_or_else(|e| {
        eprintln!("failed to map {}: {e}", parsed.iso_path.display());
        std::process::exit(1);
    });
    // Vault before install: a missing one should not cost a full image
    // read first.
    let keys = KeyVault::load_for_vfs(&parsed.output_dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    println!(
        "cellgov_install: installing disc from {} ({:.1} MB)",
        parsed.iso_path.display(),
        iso_data.len() as f64 / (1024.0 * 1024.0)
    );

    let bar = ProgressBar::start(
        parsed.render.caps(),
        &INSTALL_TASK,
        &container_label(&parsed.iso_path),
    );
    let reporter = bar.sink();
    let outcome = game_install::install_iso(
        &iso_data,
        &keys,
        &parsed.output_dir,
        InstallOptions {
            force: parsed.force,
            progress: &*reporter,
        },
    );
    // Bar down first: the render thread owns stderr while it runs, and
    // its next frame would cursor-up over the failure line below.
    let outcome = match outcome {
        Ok(o) => {
            bar.finish();
            o
        }
        Err(e) => {
            bar.abort();
            eprintln!("install-iso failed: {e}");
            std::process::exit(1);
        }
    };

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

    let opts = game_uninstall::UninstallOptions {
        verify: parsed.verify,
        keep_rap: parsed.keep_rap,
        force: parsed.force,
    };
    let outcome = game_uninstall::uninstall(&parsed.title_id, &parsed.output_dir, opts)
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
    // Non-zero only under --force, which is the one way a divergence
    // gets past the gate; the override still names what it waved past.
    if let Some(n) = outcome.files_diverged {
        if n > 0 {
            eprintln!("  --force overrode {n} recorded files that were missing or modified");
        }
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
/// decrypt ELF bytes. A file that fails to decrypt (e.g. a revision
/// with no APP key) is left out of the manifest, as is a file that is
/// neither an SCE container nor a bare ELF. Each omission is named on
/// stderr with the reason -- a tally alone cannot distinguish an
/// expected missing-key skip from a corrupt install.
///
/// # Errors
///
/// [`FirmwareCliError::FirmwareTreeReadFailed`] when the walk cannot
/// list part of the tree, plus the per-file read / strip_prefix /
/// non-UTF-8 refusals.
#[cfg(feature = "decrypt")]
fn build_firmware_manifest(
    pup_data: &[u8],
    pup_image_version: u64,
    output_dir: &Path,
    keys: &KeyVault,
) -> Result<FirmwareManifest, FirmwareCliError> {
    let mut pup_hasher = Sha256::new();
    pup_hasher.update(pup_data);
    let pup_sha256 = manifest::Sha256(pup_hasher.finalize().into());

    let mut sprx_paths = Vec::new();
    collect_sprx_paths(output_dir, &mut sprx_paths)?;

    let mut files = Vec::with_capacity(sprx_paths.len());
    let mut undecryptable: Vec<(PathBuf, String)> = Vec::new();
    let mut not_a_module: Vec<(PathBuf, usize)> = Vec::new();
    for sprx_path in &sprx_paths {
        let raw = std::fs::read(sprx_path).map_err(|source| FirmwareCliError::SprxReadFailed {
            path: sprx_path.clone(),
            source,
        })?;
        let (elf, revision) = if self_image::is_sce_wrapped(&raw) {
            let elf = match sce::decrypt_self_to_elf(&raw, keys) {
                Ok(e) => e,
                // The module is omitted from the manifest either way,
                // but the omission carries its cause: a bare tally
                // cannot tell a revision with no APP key from a
                // truncated or corrupted install.
                Err(source) => {
                    undecryptable.push((sprx_path.clone(), source.to_string()));
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
        } else if raw.starts_with(&ELF_MAGIC) {
            // Pre-decrypted `.prx` files carry no SCE wrapper: hash the
            // raw bytes (identical to their post-decrypt image) and
            // record revision 0, since the wrapper that carried it is
            // gone.
            (raw, 0)
        } else {
            // Neither an SCE container nor a bare ELF, so there is no
            // module image to hash. PS3 firmware ships at least one
            // zero-byte `.sprx` placeholder; recording it would put the
            // empty-bytes hash in the manifest under revision 0, as
            // though an empty file were a pre-decrypted module the boot
            // verifier could load.
            not_a_module.push((sprx_path.clone(), raw.len()));
            continue;
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
    if !undecryptable.is_empty() {
        eprintln!(
            "  ({} SPRX omitted from the manifest: undecryptable)",
            undecryptable.len()
        );
        for (path, why) in &undecryptable {
            eprintln!("    {}: {why}", path.display());
        }
    }
    if !not_a_module.is_empty() {
        eprintln!(
            "  ({} .sprx/.prx omitted from the manifest: neither an SCE container nor an ELF)",
            not_a_module.len()
        );
        for (path, len) in &not_a_module {
            eprintln!("    {} ({len} bytes)", path.display());
        }
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

/// Parsed `keys` subcommand arguments.
#[derive(Debug, PartialEq, Eq)]
enum KeysCommand {
    /// Inventory of the vault at `path`, or of the one the decrypting
    /// subcommands would read for `vfs_root`.
    Show {
        path: Option<PathBuf>,
        vfs_root: PathBuf,
    },
    /// Normalize `path` into `<vfs_root>/.cellgov/keys/keys.toml`.
    Import {
        path: PathBuf,
        vfs_root: PathBuf,
        replace: bool,
    },
    /// Delete `<vfs_root>/.cellgov/keys/`.
    Remove { vfs_root: PathBuf },
}

fn parse_keys_args(args: &[String]) -> Result<KeysCommand, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::KeysMissingSubcommand);
    }
    let subcommand: &'static str = match args[2].as_str() {
        "show" => "show",
        "import" => "import",
        "remove" => "remove",
        other => return Err(FirmwareCliError::KeysUnknownSubcommand(other.to_string())),
    };
    let mut positional: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut replace = false;
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
            "--replace" if subcommand == "import" => replace = true,
            other if other.starts_with("--") => {
                return Err(FirmwareCliError::UnknownArgument(other.to_string()));
            }
            other if subcommand == "remove" || positional.is_some() => {
                return Err(FirmwareCliError::KeysExtraPositional {
                    subcommand,
                    extra: other.to_string(),
                });
            }
            other => positional = Some(PathBuf::from(other)),
        }
        i += 1;
    }
    let vfs_root = output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    Ok(match subcommand {
        "show" => KeysCommand::Show {
            path: positional,
            vfs_root,
        },
        "import" => KeysCommand::Import {
            path: positional.ok_or(FirmwareCliError::KeysImportMissingPath)?,
            vfs_root,
            replace,
        },
        _ => KeysCommand::Remove { vfs_root },
    })
}

fn cmd_keys(args: &[String]) {
    let parsed = parse_keys_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });
    match parsed {
        KeysCommand::Show { path, vfs_root } => {
            let location = match path {
                Some(p) => p,
                None => KeyVault::locate_from(std::env::var_os(ENV_KEYS), &vfs_root)
                    .unwrap_or_else(|e| {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }),
            };
            let vault = KeyVault::load_from_path(&location).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            print!("{}", render_key_inventory(&location, &vault));
            if !vault.missing_for_decrypt().is_empty() {
                std::process::exit(2);
            }
        }
        KeysCommand::Import {
            path,
            vfs_root,
            replace,
        } => {
            let file = installed_keys_dir(&vfs_root).join(INSTALLED_KEYS_FILE);
            // Decided before the import writes: "merged" is only true
            // when a vault was already there to merge into.
            let merged = !replace && file.is_file();
            let vault = import_keys(&path, &vfs_root, replace).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            println!(
                "cellgov_install: {} {} into {}",
                if merged { "merged" } else { "wrote" },
                path.display(),
                file.display()
            );
            println!("  {}", vault.summary());
            for ignored in vault.ignored() {
                println!("  set aside {}: {}", ignored.at, ignored.reason);
            }
            let missing = vault.missing_for_decrypt();
            if missing.is_empty() {
                println!("  decrypt paths: ready");
            } else {
                println!("  missing for decrypt: {}", missing.join(", "));
            }
        }
        KeysCommand::Remove { vfs_root } => {
            let dir = installed_keys_dir(&vfs_root);
            match remove_keys(&vfs_root) {
                Ok(true) => println!("cellgov_install: removed {}", dir.display()),
                Ok(false) => println!("cellgov_install: nothing installed at {}", dir.display()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// The `keys show` report for the vault loaded from `location`.
fn render_key_inventory(location: &Path, vault: &KeyVault) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("key vault: {}", location.display()));
    lines.push(format!("  sources: {} file(s)", vault.sources().len()));
    for slot in Slot::ALL {
        lines.push(match vault.slot_provenance(slot) {
            Some(at) => format!("  {}: {} bytes, from {at}", slot.name(), slot.byte_len()),
            None => format!("  {}: missing", slot.name()),
        });
    }
    let scepkg = vault.scepkg_keys().map(Iterator::count).unwrap_or(0);
    lines.push(format!("  scepkg: {scepkg} keyset(s)"));
    for class in [SelfClass::App, SelfClass::Npdrm] {
        let revisions: Vec<String> = vault
            .labeled_revisions(class)
            .map(|r| format!("0x{r:04x}"))
            .collect();
        let labeled = if revisions.is_empty() {
            "(none)".to_string()
        } else {
            revisions.join(", ")
        };
        lines.push(format!(
            "  {class}: revisions {labeled}, {} unlabeled",
            vault.unlabeled_count(class)
        ));
    }
    if !vault.ignored().is_empty() {
        lines.push("  set aside:".to_string());
        for ignored in vault.ignored() {
            lines.push(format!("    {}: {}", ignored.at, ignored.reason));
        }
    }
    let missing = vault.missing_for_decrypt();
    if missing.is_empty() {
        lines.push("  decrypt paths: ready".to_string());
    } else {
        lines.push(format!("  missing for decrypt: {}", missing.join(", ")));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Whether `vault` holds any value a decrypt path could use.
fn holds_any_key(vault: &KeyVault) -> bool {
    Slot::ALL
        .iter()
        .any(|s| vault.slot_provenance(*s).is_some())
        || vault.scepkg_keys().is_ok()
        || [SelfClass::App, SelfClass::Npdrm]
            .iter()
            .any(|c| vault.labeled_revisions(*c).next().is_some() || vault.unlabeled_count(*c) > 0)
}

/// Normalize the vault at `path` into `<vfs_root>/.cellgov/keys/keys.toml`,
/// merging into the vault already there unless `replace`.
///
/// # Errors
///
/// [`FirmwareCliError::Keys`] for a vault that will not load, or a
/// merge whose two definitions of one key disagree (the refusal names
/// both); [`FirmwareCliError::KeysNothingUsable`] when `path` held no
/// key at all; and the directory / file write refusals.
fn import_keys(path: &Path, vfs_root: &Path, replace: bool) -> Result<KeyVault, FirmwareCliError> {
    let imported = KeyVault::load_from_path(path)?;
    if !holds_any_key(&imported) {
        return Err(FirmwareCliError::KeysNothingUsable {
            path: path.to_path_buf(),
        });
    }
    let dir = installed_keys_dir(vfs_root);
    let file = dir.join(INSTALLED_KEYS_FILE);
    let vault = if !replace && file.is_file() {
        let mut existing = KeyVault::load_from_path(&file)?;
        existing.merge(imported)?;
        existing
    } else {
        imported
    };
    std::fs::create_dir_all(&dir).map_err(|source| FirmwareCliError::KeysDirCreateFailed {
        path: dir.clone(),
        source,
    })?;
    std::fs::write(&file, vault.to_toml()).map_err(|source| FirmwareCliError::KeysWriteFailed {
        path: file.clone(),
        source,
    })?;
    Ok(vault)
}

/// Delete `<vfs_root>/.cellgov/keys/`; `Ok(false)` when there was
/// nothing to delete.
///
/// # Errors
///
/// [`FirmwareCliError::KeysRemoveFailed`] for any refusal other than
/// absence.
fn remove_keys(vfs_root: &Path) -> Result<bool, FirmwareCliError> {
    let dir = installed_keys_dir(vfs_root);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(FirmwareCliError::KeysRemoveFailed { path: dir, source }),
    }
}

#[cfg(test)]
mod scratch_dir;

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;
