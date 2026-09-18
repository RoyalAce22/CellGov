//! Extracts one installed firmware's stored LV2 kernel.

use std::path::{Path, PathBuf};

use cellgov_install::kernel_decrypt::{decrypt_stored_kernel, DecryptedKernel, KernelDecryptError};
use cellgov_install::keys::{version_label, KeyVault, KeyVaultError};
use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_install::store::KernelRecord;

use crate::cli::boot_cmd::DISABLE_DEFAULT_ENV;
use crate::cli::exit::die;
use crate::cli::parse::{Lv2ExtractArgs, OutputFormat};
use crate::cli::store::read::model::STORE_FORMAT_VERSION;
use crate::composition::inventory::{FirmwareEntry, InventoryError, StoreInventory};
use crate::composition::{select, FirmwareSelectError};

#[derive(Debug, serde::Serialize)]
struct Lv2ExtractDoc {
    format_version: u32,
    firmware: String,
    kernel_version: String,
    source: String,
    output: String,
    elf_bytes: usize,
    elf_sha256: String,
}

#[derive(Debug, thiserror::Error)]
enum Lv2ExtractError {
    #[error("store inventory: {0}")]
    Inventory(#[from] InventoryError),
    #[error("{0}")]
    Select(#[from] FirmwareSelectError),
    #[error(
        "no firmware is installed under {root}; install one with `cellgov firmware install \
         <PS3UPDAT.PUP>`"
    )]
    NoneInstalled { root: String },
    #[error(
        "{} firmware versions are installed under {root} ({}); name the one to extract with \
         --fw",
        installed.len(),
        installed.join(", ")
    )]
    Ambiguous {
        root: String,
        installed: Vec<String>,
    },
    #[error("firmware {version}: kernel not unpacked: {reason}")]
    NotUnpacked { version: String, reason: String },
    #[error("key vault: {0}")]
    Vault(#[from] KeyVaultError),
    #[error("firmware {version}: {source}")]
    Decrypt {
        version: String,
        #[source]
        source: KernelDecryptError,
    },
    #[error("create output directory {}: {source}", path.display())]
    CreateOutputDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("write plaintext kernel {}: {source}", path.display())]
    WriteOutput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("serialize extraction report: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub(crate) fn run(args: &Lv2ExtractArgs, vfs_flag: Option<&Path>, format: OutputFormat) {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag);
    let doc = extract(args, &vfs_root).unwrap_or_else(|e| die(&format!("lv2-extract: {e}")));
    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&doc)
                .unwrap_or_else(|e| die(&Lv2ExtractError::Serialize(e).to_string()));
            println!("{json}");
        }
        OutputFormat::Human => {
            println!(
                "firmware {} kernel {} -> {}",
                doc.firmware, doc.source, doc.output
            );
            println!(
                "  header firmware {}; {} bytes; sha256 {}",
                doc.kernel_version, doc.elf_bytes, doc.elf_sha256
            );
        }
    }
}

fn extract(args: &Lv2ExtractArgs, vfs_root: &Path) -> Result<Lv2ExtractDoc, Lv2ExtractError> {
    let store = crate::cli::keys::install_root_of(vfs_root);
    let inventory = StoreInventory::read(&store)?;
    let managed =
        select::select_firmware(&inventory, args.fw.as_deref(), None, DISABLE_DEFAULT_ENV)
            .map_err(|source| match source {
                FirmwareSelectError::NoneInstalled { root, .. } => {
                    Lv2ExtractError::NoneInstalled { root }
                }
                FirmwareSelectError::Ambiguous { root, installed } => {
                    Lv2ExtractError::Ambiguous { root, installed }
                }
                other => Lv2ExtractError::Select(other),
            })?;
    let kernel = kernel_record(&managed.entry)?;
    let vault = KeyVault::load_for_vfs(&store)?;
    let decrypted =
        decrypt_stored_kernel(&managed.entry.entry_dir, kernel, &vault).map_err(|source| {
            Lv2ExtractError::Decrypt {
                version: managed.entry.version.clone(),
                source,
            }
        })?;
    let source = stored_kernel_path(&managed.entry, kernel);
    write_output(&args.output_dir, &managed.entry.version, &source, decrypted)
}

fn kernel_record(entry: &FirmwareEntry) -> Result<&KernelRecord, Lv2ExtractError> {
    let Some(core_os) = &entry.core_os else {
        return Err(Lv2ExtractError::NotUnpacked {
            version: entry.version.clone(),
            reason: "the install record predates stored kernels".to_string(),
        });
    };
    core_os
        .kernel
        .as_ref()
        .ok_or_else(|| Lv2ExtractError::NotUnpacked {
            version: entry.version.clone(),
            reason: core_os
                .omission
                .clone()
                .unwrap_or_else(|| "the install recorded no reason".to_string()),
        })
}

fn stored_kernel_path(entry: &FirmwareEntry, kernel: &KernelRecord) -> PathBuf {
    kernel
        .path
        .split('/')
        .fold(entry.entry_dir.clone(), |path, part| path.join(part))
}

fn write_output(
    output_dir: &Path,
    firmware: &str,
    source: &Path,
    kernel: DecryptedKernel,
) -> Result<Lv2ExtractDoc, Lv2ExtractError> {
    std::fs::create_dir_all(output_dir).map_err(|source| Lv2ExtractError::CreateOutputDir {
        path: output_dir.to_path_buf(),
        source,
    })?;
    let output = output_dir.join(format!("lv2_kernel-{firmware}.elf"));
    std::fs::write(&output, &kernel.elf).map_err(|source| Lv2ExtractError::WriteOutput {
        path: output.clone(),
        source,
    })?;
    Ok(Lv2ExtractDoc {
        format_version: STORE_FORMAT_VERSION,
        firmware: firmware.to_string(),
        kernel_version: version_label(kernel.version),
        source: source.display().to_string(),
        output: output.display().to_string(),
        elf_bytes: kernel.elf.len(),
        elf_sha256: Sha256(sha256_of(&kernel.elf)).to_hex(),
    })
}

#[cfg(test)]
#[path = "tests/lv2_extract_tests.rs"]
mod tests;
