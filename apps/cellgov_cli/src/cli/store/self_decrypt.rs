//! `cellgov self decrypt`.

use std::path::Path;

use crate::cli::parse::SelfDecryptArgs;

#[cfg(feature = "decrypt")]
use crate::cli::exit::die;

#[cfg(feature = "decrypt")]
use cellgov_install::npdrm::{NpdHeaderInfo, Rap};
#[cfg(feature = "decrypt")]
use cellgov_install::{sce, self_image};
#[cfg(feature = "decrypt")]
use std::cell::RefCell;

#[cfg(not(feature = "decrypt"))]
pub(crate) fn run(_args: &SelfDecryptArgs, _vfs_root: &Path, _store: &Path) {
    crate::cli::exit::die(
        &super::StoreCliError::DecryptFeatureDisabled {
            command: "self decrypt".to_string(),
        }
        .to_string(),
    )
}

/// Write the plaintext ELF of one SELF.
///
/// - `vfs_root`: the PS3 VFS root that holds the RAP.
/// - `store`: the root that holds the key vault.
#[cfg(feature = "decrypt")]
pub(crate) fn run(args: &SelfDecryptArgs, vfs_root: &Path, store: &Path) {
    let self_path = &args.self_path;
    let output_path = args.output.clone().unwrap_or_else(|| {
        let stem = self_path.file_stem().unwrap_or_default().to_string_lossy();
        self_path.with_file_name(format!("{stem}.elf"))
    });

    let keys = super::vault_or_die(store);
    let data = std::fs::read(self_path)
        .unwrap_or_else(|e| die(&format!("failed to read {}: {e}", self_path.display())));
    println!(
        "cellgov: decrypting {} ({:.1} MB)",
        self_path.display(),
        super::megabytes(data.len()),
    );

    // `Auto` covers both classes. Firmware and disc SELFs stay
    // APP-keyed; an NPDRM title finds its klicensee the way the boot
    // path does.
    let exdata = crate::cli::title::exdata_dir(vfs_root);
    let resolve_error: RefCell<Option<super::StoreCliError>> = RefCell::new(None);
    let resolver = |npd: &NpdHeaderInfo| -> Option<Rap> {
        match super::rap::resolve(args.rap.as_deref(), &exdata, &npd.content_id) {
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

    // This check runs whichever way the decrypt went. A license-3 SELF
    // falls back to the vault's free klicensee when the resolver yields
    // no key. That fallback would otherwise hide a refused RAP behind a
    // "successful" decrypt.
    if let Some(rap_err) = resolve_error.borrow_mut().take() {
        die(&rap_err.to_string());
    }

    let elf = decrypted.unwrap_or_else(|e| {
        eprintln!("SELF decryption failed: {e}");
        // This arm is reachable only without --rap: the code above
        // already refuses an explicit RAP that will not read.
        if let sce::SceError::NoRapForNpdrmTitle { .. } = e {
            eprintln!(
                "  searched {} for <content_id>.rap; pass --rap <path> for an uninstalled title",
                exdata.display()
            );
        }
        super::super::exit::exit_failed();
    });

    std::fs::write(&output_path, &elf)
        .unwrap_or_else(|e| die(&format!("failed to write {}: {e}", output_path.display())));
    println!(
        "cellgov: wrote {} ({:.1} MB)",
        output_path.display(),
        super::megabytes(elf.len()),
    );
}
