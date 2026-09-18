//! `cellgov firmware install`.

use std::path::Path;

use cellgov_terminal::caps::RenderFlags;

use crate::cli::parse::FirmwareInstallArgs;

#[cfg(feature = "decrypt")]
use cellgov_install::firmware_install::{FirmwareInstallError, ManifestOmission, PackageSummary};
#[cfg(feature = "decrypt")]
use cellgov_install::progress::FIRMWARE_TASK;
#[cfg(feature = "decrypt")]
use cellgov_install::store::CoreOsRecord;
#[cfg(feature = "decrypt")]
use cellgov_terminal::progress::ProgressBar;

#[cfg(feature = "decrypt")]
use super::{container_label, install_caps, map_container_or_die, megabytes, vault_or_die};

#[cfg(not(feature = "decrypt"))]
pub(crate) fn install(_args: &FirmwareInstallArgs, _store: &Path, _render: RenderFlags, _v: bool) {
    crate::cli::exit::die(
        &super::StoreCliError::DecryptFeatureDisabled {
            command: "firmware install".to_string(),
        }
        .to_string(),
    )
}

/// Install system software from a PUP into `store`, or with
/// `--kernel-only` add the kernel to the entry the PUP already
/// installed.
#[cfg(feature = "decrypt")]
pub(crate) fn install(
    args: &FirmwareInstallArgs,
    store: &Path,
    render: RenderFlags,
    verbose: bool,
) {
    // Vault before container: a missing one should not cost a full PUP
    // read first.
    let keys = vault_or_die(store);
    let pup_data = map_container_or_die(&args.path);

    if args.kernel_only {
        println!(
            "cellgov: unpacking the kernel from {} ({:.1} MB) into its installed entry",
            args.path.display(),
            megabytes(pup_data.len()),
        );
        let outcome =
            cellgov_install::firmware_install::complete_kernel(&pup_data, &keys, store, &())
                .unwrap_or_else(|e| {
                    eprintln!("install --kernel-only failed: {e}");
                    std::process::exit(1);
                });
        println!(
            "  firmware {}: entry {}",
            outcome.version,
            outcome.entry_dir.display()
        );
        println!("  record {}", outcome.record_path.display());
        if outcome.replaced {
            println!("  the kernel already stored there was written over");
        }
        report_core_os(&outcome.core_os);
        return;
    }

    println!(
        "cellgov: installing firmware from {} ({:.1} MB)",
        args.path.display(),
        megabytes(pup_data.len()),
    );

    let bar = ProgressBar::start(
        install_caps(render),
        &FIRMWARE_TASK,
        &container_label(&args.path),
    );
    let reporter = bar.sink();
    let outcome = cellgov_install::firmware_install::install_pup(
        &pup_data, &keys, store, args.force, &*reporter,
    );
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
    report_core_os(&outcome.core_os);
    super::report_rename_retries(outcome.rename_retries);
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
pub(super) fn report_omissions(omissions: &[ManifestOmission]) {
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

/// Report what the CoreOS package yielded: the stored kernel, or why
/// there is none.
#[cfg(feature = "decrypt")]
pub(super) fn report_core_os(core_os: &CoreOsRecord) {
    match (&core_os.kernel, &core_os.omission) {
        (Some(kernel), _) => println!(
            "  kernel {} (as stored, sha256 {}; {} in the CoreOS table)",
            kernel.path,
            kernel.stored_sha256.to_hex(),
            plural(core_os.files.len(), "file", "files"),
        ),
        (None, Some(why)) => {
            eprintln!("  kernel not unpacked (the dev_flash tree is unaffected): {why}");
        }
        (None, None) => eprintln!("  kernel not unpacked (the dev_flash tree is unaffected)"),
    }
}

/// `1 file` / `3 files`, so a count of one does not read as `1 file(s)`.
#[cfg(feature = "decrypt")]
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(feature = "decrypt")]
fn report_install_failure(e: &FirmwareInstallError) {
    eprintln!("install failed: {e}");
    for line in install_failure_detail(e) {
        eprintln!("  {line}");
    }
}

/// The per-package and per-entry failures behind `e`, one line each.
///
/// `StagingResidue` renders only the summary counts of the fault it
/// wraps, so this function reads the detail from that wrapped cause.
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

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;
