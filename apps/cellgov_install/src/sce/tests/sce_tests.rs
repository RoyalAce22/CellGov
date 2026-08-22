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

/// Minimal SCE buffer with a program identification header at
/// `pid_off` whose first u64 is `authid`.
fn build_self_with_authid(pid_off: u64, authid: u64, len: usize) -> Vec<u8> {
    let mut data = vec![0u8; len];
    data[0..4].copy_from_slice(&0x53434500u32.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&pid_off.to_be_bytes());
    let off = pid_off as usize;
    if off + 8 <= len {
        data[off..off + 8].copy_from_slice(&authid.to_be_bytes());
    }
    data
}

#[test]
fn parse_program_authority_id_reads_the_pid_header_first_u64() {
    let data = build_self_with_authid(0x70, 0x1010_0000_0100_0003, 0x100);
    assert_eq!(
        parse_program_authority_id(&data).unwrap(),
        0x1010_0000_0100_0003
    );
}

#[test]
fn parse_program_authority_id_rejects_non_sce_input() {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x7F45_4C46u32.to_be_bytes()); // raw ELF magic
    assert!(matches!(
        parse_program_authority_id(&data).unwrap_err(),
        SceError::BadMagic { .. }
    ));
}

#[test]
fn parse_program_authority_id_rejects_out_of_range_offset() {
    let data = build_self_with_authid(0x1000, 0, 0x100);
    assert!(matches!(
        parse_program_authority_id(&data).unwrap_err(),
        SceError::HeaderOffsetOutOfRange { .. }
    ));
}

#[test]
fn parse_program_authority_id_rejects_truncated_ext_header() {
    let mut data = vec![0u8; 0x24];
    data[0..4].copy_from_slice(&0x53434500u32.to_be_bytes());
    assert!(matches!(
        parse_program_authority_id(&data).unwrap_err(),
        SceError::TooSmall { .. }
    ));
}

/// Hand-verified ground truth (independent byte-level parse of the
/// plaintext headers): flOw (NPDRM, program_type 8) and WipEout (disc
/// APP, program_type 4) both carry the retail-application authority id
/// `0x1010_0000_0100_0003`.
#[test]
fn parse_program_authority_id_matches_known_corpus_values() {
    let root = {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p
    };
    // (label, fixture, the directory `install-game` / `install-iso`
    // creates for that title, expected authority id).
    let cases = [
        (
            "flOw (NPDRM SELF)",
            "vfs/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN",
            "vfs/dev_hdd0/game/NPUA80001",
            0x1010_0000_0100_0003u64,
        ),
        (
            "WipEout (disc SELF)",
            "vfs/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.BIN",
            "vfs/dev_bdvd/BCES00664",
            0x1010_0000_0100_0003u64,
        ),
    ];
    for (label, rel, installed, expected) in cases {
        let path = root.join(rel);
        let Ok(bytes) = std::fs::read(&path) else {
            assert!(
                !root.join(installed).is_dir(),
                "{label} is installed under {installed} but the pin {rel} \
                 resolved nothing: the path pin is stale and proves nothing",
            );
            eprintln!("parse_program_authority_id corpus pin: skipping {label} (not installed)");
            continue;
        };
        assert_eq!(
            parse_program_authority_id(&bytes).unwrap(),
            expected,
            "{label}: authority id mismatch",
        );
    }
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
fn a_null_section_header_offset_does_not_overwrite_the_elf_header() {
    let mut data = build_synthetic_self();
    // Section-header table present in the SELF (shdr_offset_in_self at
    // +0x40) and e_shnum > 0, but the inner ELF declares e_shoff = 0.
    // Placing the table at 0 would land on the ELF header.
    data[0x40..0x48].copy_from_slice(&0x300u64.to_be_bytes());
    data[0x13C..0x13E].copy_from_slice(&1u16.to_be_bytes());
    let elf = assemble_elf_from_sections(&data, &[]).expect("null e_shoff drops the table");
    assert_eq!(
        &elf[0..4],
        &0x7F45_4C46u32.to_be_bytes(),
        "ELF header must survive"
    );
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

/// SCE buffer with a supplemental chain holding one record of `kind`
/// whose body is `body`. Chain offset/size live at 0x58 / 0x60.
fn build_self_with_supplemental(kind: u32, body: &[u8]) -> Vec<u8> {
    const CHAIN_OFF: usize = 0x100;
    let record_size = 0x10 + body.len();
    let mut data = vec![0u8; CHAIN_OFF + record_size + 0x10];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[0x58..0x60].copy_from_slice(&(CHAIN_OFF as u64).to_be_bytes());
    data[0x60..0x68].copy_from_slice(&(record_size as u64).to_be_bytes());
    data[CHAIN_OFF..CHAIN_OFF + 4].copy_from_slice(&kind.to_be_bytes());
    data[CHAIN_OFF + 4..CHAIN_OFF + 8].copy_from_slice(&(record_size as u32).to_be_bytes());
    data[CHAIN_OFF + 0x10..CHAIN_OFF + 0x10 + body.len()].copy_from_slice(body);
    data
}

#[test]
fn parse_control_flags1_reads_the_capability_body_first_word() {
    let mut body = vec![0u8; 0x20];
    body[0..4].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    let data = build_self_with_supplemental(1, &body);
    assert_eq!(parse_control_flags1(&data).unwrap(), Some(0x4000_0000));
}

#[test]
fn parse_control_flags1_is_none_when_no_capability_record_is_present() {
    // A type-3 (NPDRM) record only: the chain exists but carries no
    // capability header, which is the unprivileged retail shape.
    let data = build_self_with_supplemental(3, &[0u8; 0x20]);
    assert_eq!(parse_control_flags1(&data).unwrap(), None);
}

#[test]
fn parse_control_flags1_rejects_a_body_too_short_for_the_flags_word() {
    let data = build_self_with_supplemental(1, &[0u8; 2]);
    assert!(
        matches!(
            parse_control_flags1(&data).unwrap_err(),
            SceError::HeaderOffsetOutOfRange { .. }
        ),
        "a 2-byte capability body cannot hold the flags word"
    );
}

#[test]
fn parse_control_flags1_rejects_non_sce_input() {
    let data = vec![0u8; 0x200];
    assert!(matches!(
        parse_control_flags1(&data).unwrap_err(),
        SceError::BadMagic { .. }
    ));
}

/// Corpus pin for the privilege split this slice exists to produce:
/// vsh.self is root-capable, retail application SELFs are not.
#[test]
fn parse_control_flags1_matches_known_corpus_values() {
    let root = {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p
    };
    // (label, fixture, the directory `cellgov_install` creates when
    // that module / title is installed, expected ctrl_flags1).
    let cases = [
        (
            "vsh.self (CoreOS)",
            "vfs/dev_flash/vsh/module/vsh.self",
            "vfs/dev_flash/vsh/module",
            0x4000_0000u32,
        ),
        (
            "flOw (NPDRM SELF)",
            "vfs/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN",
            "vfs/dev_hdd0/game/NPUA80001",
            0x0000_0000u32,
        ),
        (
            "Super Stardust HD (NPDRM SELF)",
            "vfs/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN",
            "vfs/dev_hdd0/game/NPUA80068",
            0x0000_0000u32,
        ),
    ];
    let mut checked = 0;
    for (label, rel, installed, expected) in cases {
        let path = root.join(rel);
        let Ok(bytes) = std::fs::read(&path) else {
            assert!(
                !root.join(installed).is_dir(),
                "{label} is installed under {installed} but the pin {rel} \
                 resolved nothing: the path pin is stale and proves nothing",
            );
            eprintln!("parse_control_flags1 corpus pin: skipping {label} (not installed)");
            continue;
        };
        let got = parse_control_flags1(&bytes).unwrap().unwrap_or(0);
        assert_eq!(got, expected, "{label}: ctrl_flags1 mismatch");
        // The whole point of the value: root for vsh, not for games.
        assert_eq!(
            got & 0xC000_0000 != 0,
            expected != 0,
            "{label}: root predicate disagrees with the pinned flags"
        );
        checked += 1;
    }
    eprintln!("parse_control_flags1 corpus pin: checked {checked}/3 fixtures");
}
