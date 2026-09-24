//! `cellgov self decrypt`.

use std::path::Path;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::SelfDecryptArgs;

#[cfg(feature = "decrypt")]
use cellgov_install::store::hdd0_exdata_dir;
#[cfg(feature = "decrypt")]
use cellgov_install::{npdrm, sce, self_image};

#[cfg(not(feature = "decrypt"))]
pub(crate) fn run(
    _args: &SelfDecryptArgs,
    _vfs_root: &Path,
    _store: &Path,
) -> Result<CommandExitCode, CommandError> {
    Err(CommandError::failed(
        super::StoreCliError::DecryptFeatureDisabled {
            command: "self decrypt".to_string(),
        }
        .to_string(),
    ))
}

/// Write the plaintext ELF of one SELF.
///
/// - `vfs_root`: the PS3 VFS root that holds the RAP.
/// - `store`: the root that holds the key vault.
#[cfg(feature = "decrypt")]
pub(crate) fn run(
    args: &SelfDecryptArgs,
    vfs_root: &Path,
    store: &Path,
) -> Result<CommandExitCode, CommandError> {
    let self_path = &args.self_path;
    let output_path = args.output.clone().unwrap_or_else(|| {
        let stem = self_path.file_stem().unwrap_or_default().to_string_lossy();
        self_path.with_file_name(format!("{stem}.elf"))
    });

    let keys = super::vault(store)?;
    let data = std::fs::read(self_path).map_err(|error| {
        CommandError::failed(format!("failed to read {}: {error}", self_path.display()))
    })?;
    println!(
        "cellgov: decrypting {} ({:.1} MB)",
        self_path.display(),
        super::megabytes(data.len()),
    );

    // `Auto` covers both classes. Firmware and disc SELFs stay
    // APP-keyed; an NPDRM title finds its klicensee the way the boot
    // path does. The NPD header is plaintext, so the RAP is resolved
    // before the decrypt, and a refused RAP exits naming the file
    // rather than the license-3 free-key fallback succeeding.
    let exdata = hdd0_exdata_dir(vfs_root);
    let rap = match npdrm::find_npd_header_info(&data) {
        Ok(Some(npd)) if self_image::is_sce_wrapped(&data) => {
            super::rap::resolve(args.rap.as_deref(), &exdata, &npd.content_id)
                .map_err(|e| CommandError::failed(e.to_string()))?
        }
        // A plaintext image and an APP-keyed SELF ask for no RAP, and a
        // chain that will not walk is the decrypt's own refusal.
        Ok(Some(_) | None) | Err(_) => None,
    };
    let lookup = |_: &npdrm::NpdHeaderInfo| Ok(rap);

    let decrypted =
        self_image::to_plaintext_elf(&data, &keys, self_image::KeyPolicy::Auto(&lookup));

    let elf = match decrypted {
        Ok(elf) => elf,
        Err(error) => {
            eprintln!("SELF decryption failed: {error}");
            // Without --rap, this arm can report an automatic RAP lookup failure.
            // The explicit RAP path returns its read error above.
            if let sce::SceError::NoRapForNpdrmTitle { .. } = error {
                eprintln!(
                "  searched {} for <content_id>.rap; pass --rap <path> for an uninstalled title",
                exdata.display()
            );
            }
            return Ok(CommandExitCode::new(crate::cli::exit_codes::FAILED));
        }
    };

    std::fs::write(&output_path, &elf).map_err(|error| {
        CommandError::failed(format!(
            "failed to write {}: {error}",
            output_path.display()
        ))
    })?;
    println!(
        "cellgov: wrote {} ({:.1} MB)",
        output_path.display(),
        super::megabytes(elf.len()),
    );
    Ok(CommandExitCode::SUCCESS)
}
