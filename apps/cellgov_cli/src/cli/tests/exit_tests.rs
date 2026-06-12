//! PPU-image decrypt helper: plaintext passthrough without a RAP
//! lookup.

use super::*;

#[test]
fn decrypt_passes_plaintext_elf_through_unchanged() {
    // Non-SCE bytes (ELF magic) are returned verbatim without a RAP
    // lookup, so this never touches the filesystem.
    let mut elf = vec![0x7F, b'E', b'L', b'F'];
    elf.extend_from_slice(&[0u8; 60]);
    let out = decrypt_ppu_self_or_die(&elf, "fixture.elf", Path::new("/nonexistent"));
    assert_eq!(out, elf);
}

#[test]
fn decrypt_passes_short_non_sce_bytes_through() {
    let bytes = vec![1u8, 2, 3];
    let out = decrypt_ppu_self_or_die(&bytes, "x", Path::new("/nonexistent"));
    assert_eq!(out, bytes);
}
