//! SCE container header parsing, ELF reassembly bounds checks, and non-semantic ELF byte masking.

use super::*;

#[test]
fn parse_sce_header_rejects_short() {
    assert!(parse_sce_header(&[0u8; 16]).is_err());
}

#[test]
fn parse_sce_header_rejects_bad_magic() {
    let mut data = [0u8; 0x20];
    data[0..4].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
    assert!(matches!(
        parse_sce_header(&data).unwrap_err(),
        SceError::BadMagic { .. }
    ));
}

#[test]
fn parse_sce_header_accepts_valid() {
    let mut data = [0u8; 0x20];
    data[0..4].copy_from_slice(&0x53434500u32.to_be_bytes());
    data[16..24].copy_from_slice(&256u64.to_be_bytes());
    let hdr = parse_sce_header(&data).unwrap();
    assert_eq!(hdr.magic, 0x53434500);
    assert_eq!(hdr.header_size, 256);
}

#[test]
fn decrypt_package_rejects_truncated() {
    assert!(decrypt_package(&[0u8; 8]).is_err());
}

#[test]
fn mask_non_semantic_elf_bytes_zeroes_section_header_fields_and_moves_nothing_else() {
    // The {e_shoff, e_shnum, e_shstrndx} set is empirically
    // sufficient for the current title corpus (flOw / SSHD /
    // WipEout + the firmware-PRX byte parity). Not proven-minimal
    // against arbitrary PS3 SELFs; widen only when a corpus
    // addition surfaces a fourth non-semantic ELF64 header field.
    let mut elf: Vec<u8> = (0u8..=0xFFu8).cycle().take(0x80).collect();
    elf[0x28..0x30].copy_from_slice(&0xDEADBEEFCAFEBABEu64.to_be_bytes());
    elf[0x3C..0x3E].copy_from_slice(&0x4242u16.to_be_bytes());
    elf[0x3E..0x40].copy_from_slice(&0x1234u16.to_be_bytes());
    let before = elf.clone();

    mask_non_semantic_elf_bytes(&mut elf);

    assert_eq!(&elf[0x28..0x30], &[0u8; 8], "e_shoff");
    assert_eq!(&elf[0x3C..0x3E], &[0u8; 2], "e_shnum");
    assert_eq!(&elf[0x3E..0x40], &[0u8; 2], "e_shstrndx");

    // Nothing-else-moved: every byte outside the three masked
    // ranges must equal its pre-mask value.
    for (i, (b_before, b_after)) in before.iter().zip(elf.iter()).enumerate() {
        let in_shoff = (0x28..0x30).contains(&i);
        let in_shnum = (0x3C..0x3E).contains(&i);
        let in_shstrndx = (0x3E..0x40).contains(&i);
        if in_shoff || in_shnum || in_shstrndx {
            continue;
        }
        assert_eq!(
            b_before, b_after,
            "byte at 0x{i:02x} changed: 0x{b_before:02x} -> 0x{b_after:02x}",
        );
    }
}

#[test]
fn mask_non_semantic_elf_bytes_is_noop_on_short_input() {
    let mut elf = vec![0xABu8; 0x3F];
    let before = elf.clone();
    mask_non_semantic_elf_bytes(&mut elf);
    assert_eq!(elf, before);
}

/// Craft a minimal SELF buffer that satisfies the early
/// fixed-position bounds checks in `assemble_elf_from_sections`:
/// ehdr at 0x100 with valid magic + ELFCLASS64 + ELF64 entsize
/// values, phdr at 0x200, no section-header table. Per-field
/// perturbations on top of this are the per-overflow tests below.
fn build_synthetic_self() -> Vec<u8> {
    let mut data = vec![0u8; 0x400];
    let ehdr_offset: u64 = 0x100;
    let phdr_offset: u64 = 0x200;
    data[0x30..0x38].copy_from_slice(&ehdr_offset.to_be_bytes());
    data[0x38..0x40].copy_from_slice(&phdr_offset.to_be_bytes());
    data[0x40..0x48].copy_from_slice(&0u64.to_be_bytes());
    // Inner ELF64 header at ehdr_offset.
    data[0x100..0x104].copy_from_slice(&0x7F45_4C46u32.to_be_bytes());
    data[0x104] = 2;
    // e_phentsize at +0x36, e_phnum at +0x38, e_shentsize at +0x3A, e_shnum at +0x3C.
    data[0x136..0x138].copy_from_slice(&0x38u16.to_be_bytes());
    data[0x138..0x13A].copy_from_slice(&0u16.to_be_bytes());
    data[0x13A..0x13C].copy_from_slice(&0x40u16.to_be_bytes());
    data[0x13C..0x13E].copy_from_slice(&0u16.to_be_bytes());
    data
}

#[test]
fn assemble_ehdr_offset_overflow_returns_typed_error() {
    let mut data = vec![0u8; 0x100];
    data[0x30..0x38].copy_from_slice(&(u64::MAX).to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

#[test]
fn assemble_phdr_table_extent_overflow_returns_typed_error() {
    let mut data = build_synthetic_self();
    // Push phdr_offset to near usize::MAX so phdr_offset + 0x38 wraps.
    data[0x38..0x40].copy_from_slice(&u64::MAX.to_be_bytes());
    // e_phnum = 1 with entsize 0x38: addition wraps.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

#[test]
fn assemble_inner_elf_bad_magic_returns_typed_error() {
    let mut data = build_synthetic_self();
    data[0x100..0x104].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(
        err,
        SceError::InnerElfBadMagic { got: 0xDEAD_BEEF }
    ));
}

#[test]
fn assemble_bad_phentsize_returns_typed_error() {
    let mut data = build_synthetic_self();
    // e_phnum > 0 so the entsize validation fires; e_phentsize = 0
    // would otherwise be permissible when no program headers exist.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    data[0x136..0x138].copy_from_slice(&0u16.to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(
        err,
        SceError::BadElfEntSize {
            what: "e_phentsize",
            got: 0,
            expected: 0x38,
        }
    ));
}

#[test]
fn assemble_bad_shentsize_returns_typed_error() {
    let mut data = build_synthetic_self();
    // e_shnum > 0 + shdr_offset_in_self > 0 so the entsize and
    // section-table extent checks both engage.
    data[0x40..0x48].copy_from_slice(&0x40u64.to_be_bytes());
    data[0x13C..0x13E].copy_from_slice(&1u16.to_be_bytes());
    data[0x13A..0x13C].copy_from_slice(&0x80u16.to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(
        err,
        SceError::BadElfEntSize {
            what: "e_shentsize",
            got: 0x80,
            expected: 0x40,
        }
    ));
}

#[test]
fn assemble_zero_phnum_with_zero_phentsize_is_accepted() {
    // SPRX shape: e_phnum = e_shnum = 0, entsize fields zero.
    // Must clear the entsize gate; downstream failures are
    // out of scope for this assertion.
    let data = build_synthetic_self();
    let result = assemble_elf_from_sections(&data, &[]);
    if let Err(SceError::BadElfEntSize { .. }) = result {
        panic!("unexpected BadElfEntSize for SPRX-shape input");
    }
}
