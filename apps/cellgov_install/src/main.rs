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

use cellgov_install::firmware_install::{FirmwareInstallError, ManifestOmission, PackageSummary};
use cellgov_install::game_install::InstallOptions;
use cellgov_install::keys::{
    installed_keys_dir, KeyVault, KeyVaultError, SelfClass, Slot, ENV_KEYS, INSTALLED_KEYS_FILE,
};
use cellgov_install::npdrm::{NpdHeaderInfo, Rap};
use cellgov_install::progress::{FIRMWARE_TASK, INSTALL_TASK};
use cellgov_install::{firmware_install, game_install, game_uninstall, sce, self_image};
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
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
        #[cfg(feature = "decrypt")]
        "install-update" => cmd_install_update(&args),
        "uninstall" => cmd_uninstall(&args),
        "keys" => cmd_keys(&args),
        #[cfg(feature = "decrypt")]
        "decrypt-self" => cmd_decrypt_self(&args),
        #[cfg(not(feature = "decrypt"))]
        sub @ ("install" | "install-game" | "install-iso" | "install-update" | "decrypt-self") => {
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
        eprintln!("  cellgov_install install <PUP_PATH> [--output <dir>] [--force] [-v]");
        eprintln!("    [--no-progress] [--no-color] [--quiet]: progress-bar overrides");
        eprintln!("    default --output: vfs/ (at the current working directory)");
        eprintln!(
            "    extracts to firmware/<VERSION>/, keyed by the tree's own vsh/etc/version.txt"
        );
        eprintln!("    -v: also print the per-package extraction tallies");
        eprintln!("    --force: replace an already-installed firmware version");
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
        eprintln!("  cellgov_install install-update <PKG_PATH> [--output <dir>] [--force]");
        eprintln!("    [--no-progress] [--no-color] [--quiet]: progress-bar overrides");
        eprintln!("    default --output: vfs/ (at the current working directory)");
        eprintln!(
            "    takes a GD/HG update PKG; PARAM.SFO APP_VER, else VERSION, names the version"
        );
        eprintln!("    extracts to titles/<TITLE_ID>/updates/<APP_VER>/game/");
        eprintln!("    --force: replace an already-installed version of this update");
    } else {
        eprintln!("  this build has no decrypt support: only `uninstall` and `keys` run.");
        eprintln!(
            "    `install`, `install-game`, `install-iso`, `install-update` and `decrypt-self` are refused by name;"
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
    verbose: bool,
    render: RenderFlags,
}

/// `install`'s `--output` names the VFS root, the same root
/// `install-game`, `install-iso` and `install-update` populate.
const DEFAULT_INSTALL_OUTPUT: &str = cellgov_install::store::DEFAULT_VFS_ROOT;

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
    /// `install-update` invoked without an update-PKG path.
    #[error("install-update requires an update PKG path")]
    MissingUpdatePkgPath,
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
}

fn parse_install_args(args: &[String]) -> Result<InstallArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(FirmwareCliError::MissingPupPath);
    }
    let pup_path = PathBuf::from(&args[2]);
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut verbose = false;
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
            "-v" | "--verbose" => verbose = true,
            other if render.accept(other) => {}
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
        verbose,
        render,
    })
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

#[cfg(feature = "decrypt")]
fn cmd_install(args: &[String]) {
    let InstallArgs {
        pup_path,
        output_dir,
        force,
        verbose,
        render,
    } = parse_install_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    // Vault before container: a missing one should not cost a full PUP
    // read first.
    let keys = KeyVault::load_for_vfs(&output_dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    // The install reads the PUP sequentially -- tables, one HMAC pass,
    // one decrypt pass -- so a mapping's pages stream in and are
    // evicted as it goes, as install-iso's are.
    let pup_data = filebuffer::FileBuffer::open(&pup_path).unwrap_or_else(|e| {
        eprintln!("failed to map {}: {e}", pup_path.display());
        std::process::exit(1);
    });

    println!(
        "cellgov_install: installing firmware from {} ({:.1} MB)",
        pup_path.display(),
        pup_data.len() as f64 / (1024.0 * 1024.0)
    );

    let bar = ProgressBar::start(render.caps(), &FIRMWARE_TASK, &container_label(&pup_path));
    let reporter = bar.sink();
    let outcome = firmware_install::install_pup(&pup_data, &keys, &output_dir, force, &*reporter);
    // Bar down first: the render thread owns stderr while it runs, and
    // its next frame would cursor-up over the lines below.
    let outcome = match outcome {
        Ok(o) => {
            bar.finish();
            o
        }
        Err(e) => {
            bar.abort();
            report_install_failure(&e);
            std::process::exit(1);
        }
    };

    println!(
        "  firmware {}: {} files -> {}",
        outcome.version,
        outcome.files,
        outcome.entry_dir.display(),
    );
    println!(
        "  manifest {} ({} entries)",
        outcome.manifest_path.display(),
        outcome.manifest_entries,
    );
    println!("  record {}", outcome.record_path.display());
    if outcome.replaced {
        println!("  --force replaced the version that was installed there");
    }
    if verbose {
        for p in &outcome.packages {
            println!("  {}", package_summary_line(p));
        }
    }
    report_omissions(&outcome.omissions);
}

/// One `-v` line for a package: the counts it actually has.
#[cfg(feature = "decrypt")]
fn package_summary_line(p: &PackageSummary) -> String {
    let mut line = format!("{}: {} files", p.package, p.written);
    if p.pruned > 0 {
        line.push_str(&format!(", {} pruned", p.pruned));
    }
    if p.skipped > 0 {
        line.push_str(&format!(", {} entries addressing no file", p.skipped));
    }
    line
}

/// Name every module `firmware.toml` could not cover.
///
/// Printed whether or not `-v` is set: a tally alone cannot distinguish
/// an expected missing-key skip from a corrupt install.
#[cfg(feature = "decrypt")]
fn report_omissions(omissions: &[ManifestOmission]) {
    if omissions.is_empty() {
        return;
    }
    eprintln!(
        "  {} not covered by firmware.toml (the install is complete; \
         these carry no module image to hash):",
        plural(omissions.len(), "file", "files"),
    );
    for o in omissions {
        match o {
            ManifestOmission::Undecryptable { path, reason } => {
                eprintln!("    {path}: undecryptable: {reason}");
            }
            ManifestOmission::NotAModule { path, len } => {
                eprintln!("    {path}: neither an SCE container nor an ELF ({len} bytes)");
            }
        }
    }
}

/// `1 file` / `3 files`, so a count of one does not read as `1 file(s)`.
#[cfg(feature = "decrypt")]
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Report a failed firmware install, naming each package and entry a
/// partial install lost rather than only counting them.
#[cfg(feature = "decrypt")]
fn report_install_failure(e: &FirmwareInstallError) {
    eprintln!("install failed: {e}");
    for line in install_failure_detail(e) {
        eprintln!("  {line}");
    }
}

/// The per-package and per-entry failures behind `e`, one line each.
///
/// A cleanup that could not discard the staging root wraps the fault it
/// was cleaning up after, and renders only that fault's summary counts,
/// so the detail is read out of the wrapped cause instead.
#[cfg(feature = "decrypt")]
fn install_failure_detail(e: &FirmwareInstallError) -> Vec<String> {
    match e {
        FirmwareInstallError::PartialInstall {
            packages_failed,
            extract_errors,
            ..
        } => packages_failed
            .iter()
            .map(ToString::to_string)
            .chain(extract_errors.iter().map(ToString::to_string))
            .collect(),
        FirmwareInstallError::StagingResidue { cause, .. } => install_failure_detail(cause),
        // Every other variant renders its own cause inline.
        _ => Vec::new(),
    }
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

/// Parsed arguments of a subcommand taking one container path and the
/// shared output / force / progress-render flags.
struct ContainerArgs {
    path: PathBuf,
    output_dir: PathBuf,
    force: bool,
    render: RenderFlags,
}

/// Parse `<PATH> [--output <dir>] [--force]` plus the render flags,
/// reporting `missing_path` when the positional is absent.
fn parse_container_args(
    args: &[String],
    missing_path: FirmwareCliError,
) -> Result<ContainerArgs, FirmwareCliError> {
    if args.len() < 3 {
        return Err(missing_path);
    }
    let path = PathBuf::from(&args[2]);
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
    Ok(ContainerArgs {
        path,
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT)),
        force,
        render,
    })
}

fn parse_install_iso_args(args: &[String]) -> Result<ContainerArgs, FirmwareCliError> {
    parse_container_args(args, FirmwareCliError::MissingIsoPath)
}

fn parse_install_update_args(args: &[String]) -> Result<ContainerArgs, FirmwareCliError> {
    parse_container_args(args, FirmwareCliError::MissingUpdatePkgPath)
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
    let iso_data = filebuffer::FileBuffer::open(&parsed.path).unwrap_or_else(|e| {
        eprintln!("failed to map {}: {e}", parsed.path.display());
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
        parsed.path.display(),
        iso_data.len() as f64 / (1024.0 * 1024.0)
    );

    let bar = ProgressBar::start(
        parsed.render.caps(),
        &INSTALL_TASK,
        &container_label(&parsed.path),
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

#[cfg(feature = "decrypt")]
fn cmd_install_update(args: &[String]) {
    let parsed = parse_install_update_args(args).unwrap_or_else(|e| {
        eprintln!("{e}");
        print_usage();
        std::process::exit(1);
    });

    let pkg_data = filebuffer::FileBuffer::open(&parsed.path).unwrap_or_else(|e| {
        eprintln!("failed to map {}: {e}", parsed.path.display());
        std::process::exit(1);
    });
    let keys = KeyVault::load_for_vfs(&parsed.output_dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    println!(
        "cellgov_install: installing update from {} ({:.1} MB)",
        parsed.path.display(),
        pkg_data.len() as f64 / (1024.0 * 1024.0)
    );

    let bar = ProgressBar::start(
        parsed.render.caps(),
        &INSTALL_TASK,
        &container_label(&parsed.path),
    );
    let reporter = bar.sink();
    let outcome = game_install::install_update_pkg(
        &pkg_data,
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
            eprintln!("install-update failed: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "  title {} (content {}) update {}: {} files -> {}",
        outcome.title_id,
        outcome.content_id,
        outcome.version,
        outcome.file_count,
        outcome.update_dir.display(),
    );
    println!("  record {}", outcome.record_path.display());
    if outcome.replaced {
        println!("  --force replaced the version that was installed there");
    }
    if outcome.orphan {
        eprintln!(
            "  no base is installed for {}; this update patches nothing until one is",
            outcome.title_id
        );
    }
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

#[cfg(test)]
#[path = "tests/main_update_tests.rs"]
mod update_tests;
