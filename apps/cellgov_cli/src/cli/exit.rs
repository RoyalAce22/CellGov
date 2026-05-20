//! Process-exit and whole-file-read helpers shared across every
//! CLI subcommand.

use cellgov_ps3_abi::sce::SCE_MAGIC;

/// Print `msg` to stderr and exit with status 1.
pub(crate) fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

/// Read a file or die with a context-rich error.
pub(crate) fn load_file_or_die(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| die(&format!("failed to read {path}: {e}")))
}

/// Read a PPU image (raw ELF or SCE-wrapped SELF) and return plaintext
/// ELF bytes.
pub(crate) fn load_ppu_image_or_die(path: &str) -> Vec<u8> {
    let bytes = load_file_or_die(path);
    if bytes.len() >= 4 && bytes[..4] == SCE_MAGIC {
        cellgov_firmware::sce::decrypt_self_to_elf(&bytes)
            .unwrap_or_else(|e| die(&format!("failed to decrypt SELF {path}: {e}")))
    } else {
        bytes
    }
}
