//! PS3 firmware and SELF decryption tools.
//!
//! Subcommands:
//! ```text
//!   install <PUP_PATH> [--output <dir>]  -- decrypt firmware PUP
//!   decrypt-self <SELF_PATH> [--output <path>] -- decrypt SELF to ELF
//! ```

mod crypto;
mod pup;
mod sce;
mod tar;

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
    eprintln!("  cellgov_firmware install <PUP_PATH> [--output <dir>]");
    eprintln!("  cellgov_firmware decrypt-self <SELF_PATH> [--output <path>]");
}

fn cmd_decrypt_self(args: &[String]) {
    let self_path = Path::new(&args[2]);
    let output_path = if args.len() >= 5 && args[3] == "--output" {
        PathBuf::from(&args[4])
    } else {
        let stem = self_path.file_stem().unwrap_or_default().to_string_lossy();
        self_path.with_file_name(format!("{stem}.elf"))
    };

    let data = std::fs::read(self_path).unwrap_or_else(|e| {
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
    if args.len() < 3 {
        print_usage();
        std::process::exit(1);
    }

    let pup_path = Path::new(&args[2]);
    let output_dir = if args.len() >= 5 && args[3] == "--output" {
        PathBuf::from(&args[4])
    } else {
        PathBuf::from("dev_flash")
    };

    let pup_data = std::fs::read(pup_path).unwrap_or_else(|e| {
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
    let outer_tar = tar::parse(update_data);
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
    for entry in &dev_flash_entries {
        let short = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        print!("  decrypting {short}...");
        match sce::decrypt_package(&entry.data) {
            Ok(inner_tar_data) => {
                let inner_files = tar::parse(&inner_tar_data);
                if inner_files.is_empty() {
                    println!(" empty");
                    continue;
                }
                let count = tar::extract_to_disk(&inner_files, &output_dir);
                total_files += count;
                println!(" {} files", count);
            }
            Err(e) => {
                println!(" skip ({e})");
            }
        }
    }

    println!(
        "cellgov_firmware: installed {} files to {}",
        total_files,
        output_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pup_header_size_is_0x30() {
        assert_eq!(std::mem::size_of::<pup::PupHeader>(), 0x30);
    }

    #[test]
    fn pup_entry_size_is_0x20() {
        assert_eq!(std::mem::size_of::<pup::PupFileEntry>(), 0x20);
    }

    #[test]
    fn sce_header_size_is_0x20() {
        assert_eq!(std::mem::size_of::<sce::SceHeader>(), 0x20);
    }

    #[test]
    fn metadata_info_size_is_0x40() {
        assert_eq!(std::mem::size_of::<sce::MetadataInfo>(), 0x40);
    }

    #[test]
    fn metadata_header_size_is_0x20() {
        assert_eq!(std::mem::size_of::<sce::MetadataHeader>(), 0x20);
    }

    #[test]
    fn metadata_section_header_size_is_0x30() {
        assert_eq!(std::mem::size_of::<sce::MetadataSectionHeader>(), 0x30);
    }
}
