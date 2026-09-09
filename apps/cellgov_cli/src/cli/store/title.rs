//! `cellgov title install` and `cellgov title install-update`.

use std::path::Path;

use cellgov_terminal::caps::RenderFlags;

use crate::cli::parse::{InstallContainerArgs, TitleInstallArgs};

#[cfg(feature = "decrypt")]
use crate::cli::exit::die;

#[cfg(feature = "decrypt")]
use cellgov_install::container::{self, Container};
#[cfg(feature = "decrypt")]
use cellgov_install::game_install::{
    self, InstallOptions, ShippedFirmware, ShippedFirmwareDisposition,
};
#[cfg(feature = "decrypt")]
use cellgov_install::progress::INSTALL_TASK;
#[cfg(feature = "decrypt")]
use cellgov_terminal::progress::ProgressBar;

#[cfg(feature = "decrypt")]
use super::{container_label, install_caps, map_container_or_die, megabytes, vault_or_die};

#[cfg(not(feature = "decrypt"))]
pub(crate) fn install(_args: &TitleInstallArgs, _store: &Path, _render: RenderFlags) {
    crate::cli::exit::die(
        &super::StoreCliError::DecryptFeatureDisabled {
            command: "title install".to_string(),
        }
        .to_string(),
    )
}

/// Install a base title, routed by the container the file holds.
#[cfg(feature = "decrypt")]
pub(crate) fn install(args: &TitleInstallArgs, store: &Path, render: RenderFlags) {
    let data = map_container_or_die(&args.path);
    let head = &data[..data.len().min(container::SNIFF_LEN)];
    let kind = container::sniff(head).unwrap_or_else(|| {
        die(&format!(
            "title install: {} is neither a PKG (magic 0x7F PKG) nor an ISO9660 image \
             (CD001 at sector 16)",
            args.path.display(),
        ))
    });
    // The sniff alone decides this mismatch, so it comes before the
    // vault load and the RAP read. Either of those would otherwise
    // report its own failure in place of the misuse.
    if kind == Container::Iso && args.rap.is_some() {
        die("--rap names an NPDRM license; a disc image carries none");
    }
    if kind == Container::Pkg && args.no_firmware {
        die("--no-firmware declines the system software a disc image ships; a PKG ships none");
    }
    // Vault before install: a missing one should not cost a full
    // container read first.
    let keys = vault_or_die(store);
    let rap_data = args.rap.as_ref().map(|p| {
        std::fs::read(p)
            .unwrap_or_else(|e| die(&format!("failed to read RAP {}: {e}", p.display())))
    });

    println!(
        "cellgov: installing {} from {} ({:.1} MB)",
        match kind {
            Container::Pkg => "game",
            Container::Iso => "disc",
        },
        args.path.display(),
        megabytes(data.len()),
    );

    let bar = ProgressBar::start(
        install_caps(render),
        &INSTALL_TASK,
        &container_label(&args.path),
    );
    let reporter = bar.sink();
    let options = InstallOptions {
        force: args.force,
        shipped_firmware: !args.no_firmware,
        progress: &*reporter,
    };
    let outcome = match kind {
        Container::Pkg => {
            game_install::install_pkg(&data, rap_data.as_deref(), &keys, store, options)
        }
        Container::Iso => game_install::install_iso(&data, &keys, store, options),
    };
    // Bar down first: the render thread owns stderr while it runs, and
    // its next frame would cursor-up over the lines below.
    let outcome = match outcome {
        Ok(o) => {
            bar.finish();
            o
        }
        Err(e) => {
            bar.abort();
            eprintln!("title install failed: {e}");
            std::process::exit(crate::cli::exit_codes::FAILED);
        }
    };

    match kind {
        // A disc install has no NPD header and no license: the content
        // id repeats the title id, and no RAP applies.
        Container::Iso => {
            println!(
                "  title {}: {} files -> {}",
                outcome.title_id,
                outcome.file_count,
                outcome.game_dir.display(),
            );
            println!("  record {}", outcome.record_path.display());
            report_shipped_firmware(outcome.shipped_firmware.as_ref(), args.no_firmware);
        }
        Container::Pkg => {
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
            if outcome.rap_ignored {
                println!(
                    "  note: this title is not RAP-keyed (free license, or no NPD header); \
                     the supplied RAP was not installed"
                );
            }
        }
    }
    super::report_rename_retries(outcome.rename_retries);
}

#[cfg(feature = "decrypt")]
fn report_shipped_firmware(shipped: Option<&ShippedFirmware>, declined: bool) {
    let Some(shipped) = shipped else {
        if declined {
            println!(
                "  firmware: not registered (--no-firmware); the title records no shipped version"
            );
        } else {
            println!("  firmware: the disc carries no PS3_UPDATE/PS3UPDAT.PUP");
        }
        return;
    };
    match &shipped.disposition {
        ShippedFirmwareDisposition::Installed(fw) => {
            println!(
                "  firmware {} (shipped with this disc): {} files -> {}",
                fw.version,
                fw.files,
                fw.entry_dir.display(),
            );
            println!(
                "  manifest {} ({} entries), record {}",
                fw.manifest_path.display(),
                fw.manifest_entries,
                fw.record_path.display(),
            );
            super::firmware::report_omissions(&fw.omissions);
            super::report_rename_retries(fw.rename_retries);
        }
        ShippedFirmwareDisposition::AlreadyInstalled { same_pup } => {
            println!(
                "  firmware {} (shipped with this disc): already installed{}",
                shipped.version,
                if *same_pup {
                    ""
                } else {
                    ", from a different PS3UPDAT.PUP of the same version"
                },
            );
        }
    }
}

#[cfg(not(feature = "decrypt"))]
pub(crate) fn install_update(_args: &InstallContainerArgs, _store: &Path, _render: RenderFlags) {
    crate::cli::exit::die(
        &super::StoreCliError::DecryptFeatureDisabled {
            command: "title install-update".to_string(),
        }
        .to_string(),
    )
}

/// Install a GD/HG update PKG over an installed base.
#[cfg(feature = "decrypt")]
pub(crate) fn install_update(args: &InstallContainerArgs, store: &Path, render: RenderFlags) {
    let pkg_data = map_container_or_die(&args.path);
    let keys = vault_or_die(store);

    println!(
        "cellgov: installing update from {} ({:.1} MB)",
        args.path.display(),
        megabytes(pkg_data.len()),
    );

    let bar = ProgressBar::start(
        install_caps(render),
        &INSTALL_TASK,
        &container_label(&args.path),
    );
    let reporter = bar.sink();
    let outcome = game_install::install_update_pkg(
        &pkg_data,
        &keys,
        store,
        InstallOptions {
            force: args.force,
            progress: &*reporter,
            ..Default::default()
        },
    );
    let outcome = match outcome {
        Ok(o) => {
            bar.finish();
            o
        }
        Err(e) => {
            bar.abort();
            eprintln!("title install-update failed: {e}");
            std::process::exit(crate::cli::exit_codes::FAILED);
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
    super::report_rename_retries(outcome.rename_retries);
}
