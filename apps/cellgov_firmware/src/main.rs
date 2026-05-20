//! PS3 firmware and SELF decryption CLI.
//!
//! Exposes [`cellgov_firmware`]'s library as `install` and
//! `decrypt-self` subcommands.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use cellgov_firmware::manifest::{
    self, FirmwareFileEntry, FirmwareIdentity, FirmwareManifest, SUPPORTED_FORMAT_VERSION,
};
use cellgov_firmware::{pup, sce, tar};
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
        "decrypt-self" => cmd_decrypt_self(&args),
        _ => {
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  cellgov_firmware install <PUP_PATH> [--output <dir>] [--force]");
    eprintln!("    default --output: firmware/ (at the current working directory)");
    eprintln!("    --force: overwrite a non-empty output directory");
    eprintln!("  cellgov_firmware decrypt-self <SELF_PATH> [--output <path>]");
}

/// Parsed `install` subcommand arguments.
struct InstallArgs {
    pup_path: PathBuf,
    output_dir: PathBuf,
    force: bool,
}

const DEFAULT_INSTALL_OUTPUT: &str = "firmware";

/// Why a cellgov_firmware CLI helper failed.
#[derive(Debug)]
enum FirmwareCliError {
    /// `install` invoked without a PUP path.
    MissingPupPath,
    /// `decrypt-self` invoked without a SELF path.
    MissingSelfPath,
    /// `--output` flag with no following argument.
    OutputFlagMissingValue { kind: &'static str },
    /// Unknown subcommand flag.
    UnknownArgument(String),
    /// `check_output_dir`: read_dir failed on the candidate output.
    OutputDirReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `check_output_dir`: existing output is non-empty and --force not set.
    OutputDirNotEmpty { path: PathBuf },
    /// `build_firmware_manifest`: reading an SPRX failed.
    SprxReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `build_firmware_manifest`: `strip_prefix(output_dir)` failed (the
    /// path-walker produced a path that wasn't under `output_dir`, which
    /// implies a `collect_sprx_paths` bug).
    StripPrefixFailed {
        path: PathBuf,
        source: std::path::StripPrefixError,
    },
    /// `build_firmware_manifest`: an SPRX's path has non-UTF-8 bytes;
    /// firmware.toml cannot represent it.
    NonUtf8Path { path: PathBuf },
}

impl std::fmt::Display for FirmwareCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPupPath => f.write_str("install requires a PUP path"),
            Self::MissingSelfPath => f.write_str("decrypt-self requires a SELF path"),
            Self::OutputFlagMissingValue { kind } => {
                write!(f, "--output requires a {kind} argument")
            }
            Self::UnknownArgument(s) => write!(f, "unknown argument: {s}"),
            Self::OutputDirReadFailed { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::OutputDirNotEmpty { path } => write!(
                f,
                "output directory {} exists and is non-empty; pass --force to overwrite",
                path.display()
            ),
            Self::SprxReadFailed { path, source } => {
                write!(f, "read {}: {source}", path.display())
            }
            Self::StripPrefixFailed { path, source } => {
                write!(f, "strip_prefix({}): {source}", path.display())
            }
            Self::NonUtf8Path { path } => {
                write!(f, "non-utf8 firmware path: {}", path.display())
            }
        }
    }
}

impl std::error::Error for FirmwareCliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OutputDirReadFailed { source, .. } | Self::SprxReadFailed { source, .. } => {
                Some(source)
            }
            Self::StripPrefixFailed { source, .. } => Some(source),
            _ => None,
        }
    }
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
        "cellgov_firmware: decrypting {} ({:.1} MB)",
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
        "cellgov_firmware: wrote {} ({:.1} MB)",
        output_path.display(),
        elf.len() as f64 / (1024.0 * 1024.0)
    );
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

    check_output_dir(&output_dir, force).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let pup_data = std::fs::read(&pup_path).unwrap_or_else(|e| {
        eprintln!("failed to read {}: {e}", pup_path.display());
        std::process::exit(1);
    });

    println!(
        "cellgov_firmware: reading {} ({:.1} MB)",
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
    let dev_flash_entries: Vec<_> = outer_tar
        .iter()
        .filter(|e| e.name.contains("dev_flash"))
        .collect();

    println!(
        "  update_files TAR: {} entries, {} dev_flash packages",
        outer_tar.len(),
        dev_flash_entries.len()
    );

    let mut total_files = 0usize;
    let mut packages_attempted = 0usize;
    let mut extract_errors: Vec<tar::ExtractError> = Vec::new();
    for entry in &dev_flash_entries {
        packages_attempted += 1;
        let short = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        print!("  decrypting {short}...");
        match sce::decrypt_package(&entry.data) {
            Ok(inner_tar_data) => match tar::parse(&inner_tar_data) {
                Ok(inner_files) => {
                    if inner_files.is_empty() {
                        println!(" empty");
                        continue;
                    }
                    let report = tar::extract_to_disk(&inner_files, &output_dir);
                    total_files += report.written;
                    if report.errors.is_empty() {
                        println!(" {} files", report.written);
                    } else {
                        println!(
                            " {} files, {} extract errors",
                            report.written,
                            report.errors.len()
                        );
                    }
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
        eprintln!("cellgov_firmware: {} extract errors:", extract_errors.len());
        for err in &extract_errors {
            eprintln!("  {err}");
        }
    }

    if total_files == 0 {
        eprintln!(
            "cellgov_firmware: install produced 0 files (attempted {packages_attempted} dev_flash packages); refusing to claim success"
        );
        std::process::exit(1);
    }

    println!(
        "cellgov_firmware: installed {} files to {} ({} packages, {} errors)",
        total_files,
        output_dir.display(),
        packages_attempted,
        extract_errors.len(),
    );

    print!("  building firmware.toml...");
    let manifest = match build_firmware_manifest(&pup_data, pup.image_version, &output_dir) {
        Ok(m) => m,
        Err(e) => {
            println!(" FAILED ({e})");
            std::process::exit(1);
        }
    };
    let manifest_path = output_dir.join("firmware.toml");
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

/// Walk `dir` recursively in lexicographic order and append every
/// path ending in `.sprx` to `paths`.
fn collect_sprx_paths(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sorted: Vec<PathBuf> = entries.filter_map(|r| r.ok()).map(|e| e.path()).collect();
    sorted.sort();
    for p in sorted {
        if p.is_dir() {
            collect_sprx_paths(&p, paths);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        {
            paths.push(p);
        }
    }
}

/// Build the firmware.toml manifest from a freshly-installed tree.
/// PUP hash is over `pup_data`; per-file hashes are over the post-
/// decrypt ELF bytes. Files unable to decrypt are skipped silently
/// (e.g., revisions without an APP key in [`crypto::app_key_for_revision`]).
fn build_firmware_manifest(
    pup_data: &[u8],
    pup_image_version: u64,
    output_dir: &Path,
) -> Result<FirmwareManifest, FirmwareCliError> {
    let mut pup_hasher = Sha256::new();
    pup_hasher.update(pup_data);
    let pup_sha256 = manifest::Sha256(pup_hasher.finalize().into());

    let mut sprx_paths = Vec::new();
    collect_sprx_paths(output_dir, &mut sprx_paths);

    let mut files = Vec::with_capacity(sprx_paths.len());
    let mut skipped = 0usize;
    for sprx_path in &sprx_paths {
        let raw = std::fs::read(sprx_path).map_err(|source| FirmwareCliError::SprxReadFailed {
            path: sprx_path.clone(),
            source,
        })?;
        let elf = match sce::decrypt_self_to_elf(&raw) {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        // decrypt_self_to_elf already parsed the same header to get
        // here, so this parse cannot fail; expect rather than
        // silent-fallback so a future change that decouples the two
        // paths surfaces the violation instead of writing 0 (which
        // collides with the legitimate revision-0 value).
        let revision = sce::parse_sce_header(&raw)
            .expect("decrypt_self_to_elf success implies parse_sce_header success")
            .revision_flags
            & 0x7FFF;
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
            // PUP-header `image_version` is an opaque u64 identifier,
            // not a user-facing version string (RPCS3 likewise reads
            // user-facing version from `vsh/etc/version.txt`, not from
            // this header field). Render as zero-padded hex so a
            // human inspecting firmware.toml sees the raw value;
            // decimal-of-u64 would round-trip but be unreadable.
            image_version: format!("0x{pup_image_version:016x}"),
            pup_sha256,
        },
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        let mut v = vec!["cellgov_firmware".to_string(), "install".to_string()];
        v.extend(parts.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn parse_default_output_is_firmware() {
        let a = parse_install_args(&argv(&["/tmp/PS3UPDAT.PUP"])).expect("parse");
        assert_eq!(a.pup_path, PathBuf::from("/tmp/PS3UPDAT.PUP"));
        assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
        assert!(!a.force);
    }

    #[test]
    fn parse_override_output() {
        let a = parse_install_args(&argv(&["x.pup", "--output", "/elsewhere"])).expect("parse");
        assert_eq!(a.output_dir, PathBuf::from("/elsewhere"));
        assert!(!a.force);
    }

    #[test]
    fn parse_force_flag() {
        let a = parse_install_args(&argv(&["x.pup", "--force"])).expect("parse");
        assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
        assert!(a.force);
    }

    #[test]
    fn parse_force_and_output_in_either_order() {
        let a = parse_install_args(&argv(&["x.pup", "--force", "--output", "/d"]))
            .expect("parse force-first");
        assert_eq!(a.output_dir, PathBuf::from("/d"));
        assert!(a.force);

        let a = parse_install_args(&argv(&["x.pup", "--output", "/d", "--force"]))
            .expect("parse output-first");
        assert_eq!(a.output_dir, PathBuf::from("/d"));
        assert!(a.force);
    }

    #[test]
    fn parse_missing_pup_errors() {
        let r = parse_install_args(&["cellgov_firmware".into(), "install".into()]);
        assert!(r.is_err());
    }

    #[test]
    fn parse_output_without_value_errors() {
        let r = parse_install_args(&argv(&["x.pup", "--output"]));
        assert!(r.is_err());
    }

    #[test]
    fn parse_unknown_flag_errors() {
        let r = parse_install_args(&argv(&["x.pup", "--garbage"]));
        assert!(r.is_err());
    }

    #[test]
    fn check_output_dir_missing_is_ok() {
        let dir = std::env::temp_dir().join("cellgov_firmware_test_missing_xyz_31b2");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(check_output_dir(&dir, false).is_ok());
    }

    #[test]
    fn check_output_dir_empty_is_ok() {
        let dir = std::env::temp_dir().join("cellgov_firmware_test_empty_31b2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_output_dir(&dir, false).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_output_dir_nonempty_without_force_errors() {
        let dir = std::env::temp_dir().join("cellgov_firmware_test_nonempty_31b2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
        assert!(check_output_dir(&dir, false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_output_dir_nonempty_with_force_is_ok() {
        let dir = std::env::temp_dir().join("cellgov_firmware_test_force_31b2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
        assert!(check_output_dir(&dir, true).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
