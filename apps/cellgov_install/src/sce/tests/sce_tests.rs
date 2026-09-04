//! SCE container header parsing, ELF reassembly bounds checks, and non-semantic ELF byte masking.

use super::*;
#[cfg(feature = "decrypt")]
use crate::keys::{KeyVault, KeyVaultError};

#[test]
fn parse_sce_header_rejects_short() {
    assert!(matches!(
        parse_sce_header(&[0u8; 16]).unwrap_err(),
        SceError::TooSmall {
            what: "SCE header",
            got: 16,
            need: 0x20
        }
    ));
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

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_package_rejects_truncated() {
    assert!(matches!(
        decrypt_package(&[0u8; 8], &crate::test_support::synthetic_vault()).unwrap_err(),
        SceError::TooSmall {
            what: "SCE header",
            got: 8,
            need: 0x20
        }
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_package_on_a_vault_with_no_package_keyset_refuses_by_name() {
    // The header parses, so the vault is the first thing to refuse.
    let data = build_sce_container(0x0001, 0x100);
    let err = decrypt_package(&data, &KeyVault::empty()).unwrap_err();
    assert!(
        matches!(&err, SceError::Keys(e) if matches!(**e, KeyVaultError::MissingScepkg)),
        "got {err:?}"
    );
    assert!(err.to_string().contains("scepkg"), "{err}");
}

#[cfg(feature = "decrypt")]
fn keyset_toml(section: &str, label: Option<&str>, erk_byte: u8, riv_byte: u8) -> String {
    let label = label.map_or(String::new(), |l| format!("label = \"{l}\"\n"));
    let erk = format!("{erk_byte:02x}").repeat(32);
    let riv = format!("{riv_byte:02x}").repeat(16);
    format!("[[{section}]]\n{label}erk = \"{erk}\"\nriv = \"{riv}\"\n")
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_package_names_the_count_when_no_package_keyset_opens_the_envelope() {
    // A zeroed envelope decrypts to non-zero padding under any made-up
    // keyset, so both candidates are walked and neither fits.
    let toml = format!(
        "{}{}",
        keyset_toml("scepkg", None, 0x61, 0x62),
        keyset_toml("scepkg", None, 0x63, 0x64)
    );
    let keys = KeyVault::parse(std::path::Path::new("two.toml"), toml.as_bytes()).unwrap();
    let data = build_sce_container(0x0001, 0x100);
    let err = decrypt_package(&data, &keys).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoCandidateOpensEnvelope {
                class: "SCE package",
                revision: 1,
                tried: 2
            }
        ),
        "got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_self_to_elf_walks_every_unlabeled_app_candidate_before_refusing() {
    let toml = format!(
        "{}{}",
        keyset_toml("app", Some("first"), 0x71, 0x72),
        keyset_toml("app", Some("second"), 0x73, 0x74)
    );
    let keys = KeyVault::parse(std::path::Path::new("two.toml"), toml.as_bytes()).unwrap();
    // No labeled keyset for revision 2, so both unlabeled ones are
    // candidates.
    let data = build_sce_container(0x0002, 0x100);
    let err = decrypt_self_to_elf(&data, &keys).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoCandidateOpensEnvelope {
                class: "APP",
                revision: 2,
                tried: 2
            }
        ),
        "got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_self_to_elf_with_one_candidate_returns_its_own_padding_refusal() {
    let toml = keyset_toml("app", Some("only"), 0x71, 0x72);
    let keys = KeyVault::parse(std::path::Path::new("one.toml"), toml.as_bytes()).unwrap();
    let data = build_sce_container(0x0002, 0x100);
    let err = decrypt_self_to_elf(&data, &keys).unwrap_err();
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "a single candidate's refusal is not wrapped in a count, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
/// 0x100-byte SCE container with `metadata_offset` 0x20, so the key
/// envelope sits at 0x40..0x80 and the metadata directory starts at
/// 0x80.
fn build_sce_container(revision_flags: u16, header_size: u64) -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[8..10].copy_from_slice(&revision_flags.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes());
    data[16..24].copy_from_slice(&header_size.to_be_bytes());
    data
}

#[cfg(feature = "decrypt")]
#[test]
fn a_debug_container_whose_envelope_padding_is_not_zero_is_named() {
    let mut data = build_sce_container(0x8000, 0x100);
    // Envelope key-padding region at 0x40 + 0x10.
    data[0x50] = 0x01;
    let hdr = parse_sce_header(&data).unwrap();
    let err = decrypt_envelope(&data, &hdr, &[0u8; 0x20], &[0u8; 0x10], None).unwrap_err();
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "the debug branch skips the key peel, not the self-check, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_debug_container_whose_envelope_padding_is_zero_passes_through_unpeeled() {
    let mut data = build_sce_container(0x8000, 0x100);
    data[0x40..0x50].copy_from_slice(&[0xABu8; 16]);
    data[0x60..0x70].copy_from_slice(&[0xCDu8; 16]);
    let hdr = parse_sce_header(&data).unwrap();
    let envelope = decrypt_envelope(&data, &hdr, &[0u8; 0x20], &[0u8; 0x10], None)
        .expect("zero padding certifies the plaintext envelope");
    assert_eq!(&envelope[0x00..0x10], &[0xABu8; 16]);
    assert_eq!(&envelope[0x20..0x30], &[0xCDu8; 16]);
}

#[cfg(feature = "decrypt")]
#[test]
fn a_header_size_at_or_below_the_metadata_directory_start_is_not_reported_as_a_short_file() {
    // header_size 0x40 ends the directory before its 0x80 start while
    // the buffer holds 0x100 bytes -- a short-file answer here would
    // name a `need` below the bytes already on hand.
    let data = build_sce_container(0x0001, 0x40);
    let hdr = parse_sce_header(&data).unwrap();
    let err = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], None).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "SCE metadata directory"
            }
        ),
        "a header_size that describes no directory is a malformed header, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_header_size_past_the_buffer_is_still_reported_as_a_short_file() {
    let data = build_sce_container(0x0001, 0x200);
    let hdr = parse_sce_header(&data).unwrap();
    let err = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], None).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::TooSmall {
                what: "SCE metadata headers",
                got: 0x100,
                need: 0x200
            }
        ),
        "got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
fn zlib_compress(plain: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(plain).unwrap();
    encoder.finish().unwrap()
}

#[cfg(feature = "decrypt")]
/// SCE container holding one PHDR-kind, plaintext, zlib-compressed
/// section that targets program-header row 0.
///
/// `decrypt_sections_from_envelope` CTR-decrypts the metadata
/// directory under the envelope it is handed; the tests hand it an
/// all-zero envelope, and CTR is its own inverse, so the directory is
/// stored through the same pass.
fn build_container_with_one_zlib_section(compressed: &[u8]) -> Vec<u8> {
    use aes::cipher::{KeyIvInit, StreamCipher};

    const DIRECTORY_OFFSET: usize = 0x80;
    const PAYLOAD_OFFSET: usize = 0x100;
    const DIRECTORY_LEN: usize = 0x20 + 0x30;
    const HEADER_SIZE: usize = DIRECTORY_OFFSET + DIRECTORY_LEN;

    let mut directory = vec![0u8; DIRECTORY_LEN];
    directory[0x0C..0x10].copy_from_slice(&1u32.to_be_bytes()); // section_count
    let row = 0x20;
    directory[row..row + 8].copy_from_slice(&(PAYLOAD_OFFSET as u64).to_be_bytes());
    directory[row + 8..row + 0x10].copy_from_slice(&(compressed.len() as u64).to_be_bytes());
    directory[row + 0x10..row + 0x14].copy_from_slice(&2u32.to_be_bytes()); // PHDR kind
    directory[row + 0x20..row + 0x24].copy_from_slice(&1u32.to_be_bytes()); // plaintext
    directory[row + 0x2C..row + 0x30].copy_from_slice(&2u32.to_be_bytes()); // zlib

    ctr::Ctr128BE::<aes::Aes128>::new(&[0u8; 16].into(), &[0u8; 16].into())
        .apply_keystream(&mut directory);

    let mut data = vec![0u8; PAYLOAD_OFFSET + compressed.len()];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes()); // metadata_offset
    data[16..24].copy_from_slice(&(HEADER_SIZE as u64).to_be_bytes());
    data[DIRECTORY_OFFSET..HEADER_SIZE].copy_from_slice(&directory);
    data[PAYLOAD_OFFSET..].copy_from_slice(compressed);
    data
}

#[cfg(feature = "decrypt")]
#[test]
fn a_zlib_section_inflating_past_its_segment_filesz_is_named() {
    let data = build_container_with_one_zlib_section(&zlib_compress(&[0xAAu8; 0x400]));
    let hdr = parse_sce_header(&data).unwrap();
    let err =
        decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], Some(&[0x100])).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionInflatesPastSegment {
                index: 0,
                prog_idx: 0,
                p_filesz: 0x100
            }
        ),
        "a stream that wants more than its segment declares must be named, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_zlib_section_inflating_to_exactly_its_segment_filesz_is_accepted() {
    let data = build_container_with_one_zlib_section(&zlib_compress(&[0xAAu8; 0x400]));
    let hdr = parse_sce_header(&data).unwrap();
    let sections = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], Some(&[0x400]))
        .expect("an exact-fit stream is the shape every real SELF ships");
    assert_eq!(sections[0].1, vec![0xAAu8; 0x400]);
}

#[cfg(feature = "decrypt")]
#[test]
fn a_zlib_section_naming_a_program_index_past_the_phdr_table_cannot_escape_the_inflate_bound() {
    let data = build_container_with_one_zlib_section(&zlib_compress(&[0xAAu8; 0x400]));
    let hdr = parse_sce_header(&data).unwrap();
    let err = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], Some(&[])).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionProgramIndexOutOfRange {
                prog_idx: 0,
                e_phnum: 0
            }
        ),
        "an out-of-range program index must be named, not read as no bound at all, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_segment_declaring_the_widest_possible_filesz_does_not_overflow_the_inflate_bound() {
    // The bound is the declared size plus one, so a `p_filesz` of
    // `usize::MAX` is the value that wraps it.
    let data = build_container_with_one_zlib_section(&zlib_compress(&[0xAAu8; 0x400]));
    let hdr = parse_sce_header(&data).unwrap();
    let sections = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], Some(&[usize::MAX]))
        .expect("a segment wider than the stream bounds nothing away");
    assert_eq!(sections[0].1.len(), 0x400);
}

#[cfg(feature = "decrypt")]
#[test]
fn a_zlib_section_in_a_container_with_no_inner_elf_inflates_unbounded() {
    // The firmware-update PKG path wraps no ELF, so no program header
    // declares a size for its sections to be held to.
    let data = build_container_with_one_zlib_section(&zlib_compress(&[0xAAu8; 0x400]));
    let hdr = parse_sce_header(&data).unwrap();
    let sections = decrypt_sections_from_envelope(&data, &hdr, &[0u8; 0x40], None)
        .expect("no segment table, no bound");
    assert_eq!(sections[0].1.len(), 0x400);
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

/// The `[title] distribution` tag that makes a base a disc tree.
const DISC_DISTRIBUTION: &str = "disc-iso";

/// The store, rooted at this workspace's VFS root.
fn corpus_layout() -> crate::store::StoreLayout {
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    crate::store::StoreLayout::new(root.join(crate::store::DEFAULT_VFS_ROOT))
}

/// The install records under one kind's directory, sorted by name.
fn records_of_kind(dir: &std::path::Path) -> Vec<crate::store::InstallRecord> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => panic!("reading the install records under {}: {e}", dir.display()),
    };
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("reading an entry of {}: {e}", dir.display()));
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".install.toml"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
            crate::store::InstallRecord::parse(&text)
                .unwrap_or_else(|e| panic!("parsing {}: {e}", p.display()))
        })
        .collect()
}

/// A title's executable, or `None` when the store holds no base record
/// for it.
///
/// # Panics
///
/// Panics when the record names a tree whose executable is gone.
fn installed_title_eboot(title_id: &str, label: &str) -> Option<std::path::PathBuf> {
    let layout = corpus_layout();
    let key = crate::store::TitleId::new(title_id).expect("a pinned title id is a store key");
    let record_path = layout.record_path(&crate::store::Artifact::TitleBase { title_id: key });
    let text = match std::fs::read_to_string(&record_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("reading {}: {e}", record_path.display()),
    };
    let record = crate::store::InstallRecord::parse(&text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", record_path.display()));
    let title = record.title.as_ref().unwrap_or_else(|| {
        panic!(
            "{} describes a {} entry, which names no title",
            record_path.display(),
            record.artifact.kind.as_str()
        )
    });
    let dir = layout.resolve_store_path(&record.artifact.store_path);
    let usrdir = if title.distribution == DISC_DISTRIBUTION {
        dir.join("PS3_GAME").join("USRDIR")
    } else {
        dir.join("USRDIR")
    };
    let eboot = usrdir.join("EBOOT.BIN");
    assert!(
        eboot.is_file(),
        "{label} is recorded but {} resolved nothing: the tree has drifted from \
         the record that names it",
        eboot.display(),
    );
    Some(eboot)
}

/// `vsh/module/vsh.self` inside every installed firmware, keyed by
/// version.
///
/// # Panics
///
/// Panics when:
/// - a record under the firmware records directory declares another
///   kind;
/// - a firmware entry holds no `vsh.self`.
fn installed_vsh_selfs() -> Vec<(String, std::path::PathBuf)> {
    let layout = corpus_layout();
    let records = layout
        .installs_dir()
        .join(crate::store::ArtifactKind::Firmware.as_str());
    records_of_kind(&records)
        .into_iter()
        .map(|record| {
            // A misfiled record would otherwise fail as a missing
            // vsh.self, which names the wrong problem.
            assert_eq!(
                record.artifact.kind,
                crate::store::ArtifactKind::Firmware,
                "install record for {} sits under the firmware records but declares {}",
                record.artifact.version,
                record.artifact.kind.as_str(),
            );
            let path = layout
                .resolve_store_path(&record.artifact.store_path)
                .join(crate::firmware_install::DEV_FLASH_MOUNT)
                .join("vsh")
                .join("module")
                .join("vsh.self");
            assert!(
                path.is_file(),
                "firmware {} is recorded but {} resolved nothing: the tree has \
                 drifted from the record that names it",
                record.artifact.version,
                path.display(),
            );
            (record.artifact.version, path)
        })
        .collect()
}

/// Hand-verified ground truth (independent byte-level parse of the
/// plaintext headers): flOw (NPDRM, program_type 8) and WipEout (disc
/// APP, program_type 4) both carry the retail-application authority id
/// `0x1010_0000_0100_0003`.
#[test]
#[cfg_attr(
    not(feature = "title-corpus"),
    ignore = "pins values read off installed titles under vfs/; run with --features title-corpus"
)]
fn parse_program_authority_id_matches_known_corpus_values() {
    let cases = [
        ("NPUA80001", "flOw (NPDRM SELF)"),
        ("BCES00664", "WipEout (disc SELF)"),
    ];
    let mut checked = 0;
    for (title_id, label) in cases {
        let Some(path) = installed_title_eboot(title_id, label) else {
            eprintln!("parse_program_authority_id corpus pin: skipping {label} (not installed)");
            continue;
        };
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        assert_eq!(
            parse_program_authority_id(&bytes).unwrap(),
            cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID,
            "{label}: authority id mismatch",
        );
        checked += 1;
    }
    // Which titles the operator owns is theirs to choose; owning none
    // makes this pin a no-op that still reports ok.
    assert!(
        checked > 0,
        "title-corpus is on but none of the {} pinned titles is installed",
        cases.len()
    );
}

#[test]
fn mask_non_semantic_elf_bytes_zeroes_section_header_fields_and_moves_nothing_else() {
    // The {e_shoff, e_shnum, e_shstrndx} set is empirically
    // sufficient for the current title corpus (flOw / SSHD /
    // WipEout + the firmware-PRX byte parity).
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

#[cfg(feature = "decrypt")]
/// Craft a minimal SELF buffer that satisfies the early
/// fixed-position bounds checks in `assemble_elf_from_sections`:
/// ehdr at 0x100 with valid magic + ELFCLASS64 + ELF64 entsize
/// values, phdr at 0x200, no section-header table.
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

#[cfg(feature = "decrypt")]
#[test]
fn assemble_ehdr_offset_overflow_returns_typed_error() {
    let mut data = vec![0u8; 0x100];
    data[0x30..0x38].copy_from_slice(&(u64::MAX).to_be_bytes());
    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
#[test]
fn the_section_header_table_lands_on_top_of_an_overlapping_segment_payload() {
    let mut data = build_synthetic_self();
    // Section-header table lives at SELF offset 0x300, one 0x40-byte
    // entry, and the inner ELF places it at e_shoff = 0x80.
    data[0x40..0x48].copy_from_slice(&0x300u64.to_be_bytes());
    data[0x128..0x130].copy_from_slice(&0x80u64.to_be_bytes());
    data[0x13C..0x13E].copy_from_slice(&1u16.to_be_bytes());
    data[0x300..0x340].fill(0x5A);
    // One program header whose segment covers exactly the same range.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    data[0x208..0x210].copy_from_slice(&0x80u64.to_be_bytes());
    data[0x220..0x228].copy_from_slice(&0x40u64.to_be_bytes());

    let sections = vec![(
        EncryptedSectionDescriptor {
            payload_offset: 0,
            payload_size: 0,
            section_kind: 2,
            program_segment_index: 0,
            sha1_hashed: 0,
            sha1_slot: 0,
            encryption_kind: 0,
            key_slot: 0,
            iv_slot: 0,
            compression_kind: 0,
        },
        vec![0xAAu8; 0x40],
    )];

    let elf = assemble_elf_from_sections(&data, &sections).expect("overlapping placement");
    assert_eq!(
        &elf[0x80..0xC0],
        &[0x5Au8; 0x40],
        "section headers must overwrite the overlapping segment payload"
    );
}

#[cfg(feature = "decrypt")]
/// PHDR-kind descriptor naming program-header row `prog_idx`. The
/// payload fields go unread: `assemble_elf_from_sections` takes the
/// already-decrypted bytes from its `sections` argument.
fn phdr_section(prog_idx: u32) -> EncryptedSectionDescriptor {
    EncryptedSectionDescriptor {
        payload_offset: 0,
        payload_size: 0,
        section_kind: 2,
        program_segment_index: prog_idx,
        sha1_hashed: 0,
        sha1_slot: 0,
        encryption_kind: 0,
        key_slot: 0,
        iv_slot: 0,
        compression_kind: 0,
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn an_empty_payload_against_a_non_zero_filesz_is_named_rather_than_left_as_zeroes() {
    let mut data = build_synthetic_self();
    // One program header: p_offset = 0x80, p_filesz = 0x40.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    data[0x208..0x210].copy_from_slice(&0x80u64.to_be_bytes());
    data[0x220..0x228].copy_from_slice(&0x40u64.to_be_bytes());

    let sections = vec![(phdr_section(0), Vec::new())];
    let err = assemble_elf_from_sections(&data, &sections).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionSizeMismatch {
                prog_idx: 0,
                got: 0,
                expected: 0x40
            }
        ),
        "an empty section for a 0x40-byte segment must be named, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_empty_payload_against_a_zero_filesz_segment_still_assembles() {
    let mut data = build_synthetic_self();
    // One program header with p_offset = 0x80 and p_filesz = 0: the
    // shape a .bss-only PT_LOAD produces, and the one case an
    // empty-payload skip would legitimately cover.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    data[0x208..0x210].copy_from_slice(&0x80u64.to_be_bytes());

    let sections = vec![(phdr_section(0), Vec::new())];
    let elf = assemble_elf_from_sections(&data, &sections).expect("zero-length segment assembles");
    assert_eq!(&elf[0..4], &0x7F45_4C46u32.to_be_bytes());
}

#[cfg(feature = "decrypt")]
#[test]
fn an_empty_payload_naming_a_program_index_past_e_phnum_is_still_rejected() {
    let mut data = build_synthetic_self();
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());

    let sections = vec![(phdr_section(7), Vec::new())];
    let err = assemble_elf_from_sections(&data, &sections).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionProgramIndexOutOfRange {
                prog_idx: 7,
                e_phnum: 1
            }
        ),
        "an empty payload must not buy a section past the phdr table, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_program_segment_extent_too_large_to_allocate_is_named_rather_than_aborting() {
    let mut data = build_synthetic_self();
    // One program header whose p_offset alone names an image past
    // anything a host can back. The extent addition does not overflow,
    // so only the fallible reserve stands between this header and an
    // allocation abort.
    data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
    data[0x208..0x210].copy_from_slice(&0x0000_7FFF_FFFF_0000u64.to_be_bytes());
    data[0x220..0x228].copy_from_slice(&0x40u64.to_be_bytes());

    let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
    assert!(
        matches!(err, SceError::ReconstructedElfTooLarge { .. }),
        "an unallocatable image size must be named, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
#[test]
fn an_sprx_shape_with_zero_counts_and_zero_entsizes_assembles_to_a_bare_elf_header() {
    // SPRX shape: e_phnum = e_shnum = 0, entsize fields zero. The
    // entsize gate fires only on a non-zero count, so this clears it
    // and reassembles to the 0x40-byte header alone.
    let data = build_synthetic_self();
    let elf = assemble_elf_from_sections(&data, &[]).expect("SPRX-shape input reassembles");
    assert_eq!(
        elf.len(),
        0x40,
        "no phdr table, no payload sections, no shdr table"
    );
    assert_eq!(&elf[0..4], &0x7F45_4C46u32.to_be_bytes());
    assert_eq!(
        &elf[0x20..0x28],
        &0x40u64.to_be_bytes(),
        "e_phoff rewritten to the packed phdr position"
    );
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

/// The `ctrl_flags1` word a root-capable SELF carries.
const CTRL_FLAGS1_ROOT: u32 = 0x4000_0000;

/// Corpus pin for the privilege split: vsh.self is root-capable,
/// retail application SELFs are not.
#[test]
#[cfg_attr(
    not(feature = "title-corpus"),
    ignore = "pins values read off an installed vfs/; run with --features title-corpus"
)]
fn parse_control_flags1_matches_known_corpus_values() {
    let retail = [
        ("NPUA80001", "flOw (NPDRM SELF)"),
        ("NPUA80068", "Super Stardust HD (NPDRM SELF)"),
    ];
    let mut pins: Vec<(String, std::path::PathBuf, u32)> = installed_vsh_selfs()
        .into_iter()
        .map(|(version, path)| {
            (
                format!("vsh.self (CoreOS, firmware {version})"),
                path,
                CTRL_FLAGS1_ROOT,
            )
        })
        .collect();
    pins.extend(retail.into_iter().filter_map(|(title_id, label)| {
        installed_title_eboot(title_id, label).map(|path| (label.to_string(), path, 0))
    }));
    // Floor only. Proving the root-vs-non-root split needs one fixture
    // of each class, but vsh comes with firmware while the games come
    // with titles -- two independent features -- so a run carrying only
    // one class still checks what it can rather than failing.
    assert!(
        !pins.is_empty(),
        "title-corpus is on but the store holds neither a firmware nor a pinned title"
    );
    for (label, path, expected) in &pins {
        let bytes =
            std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let flags = parse_control_flags1(&bytes).unwrap();
        // Every fixture carries a capability record; the retail pair's
        // flags word is zero.
        assert!(flags.is_some(), "{label}: capability-record presence");
        assert_eq!(flags.unwrap_or(0), *expected, "{label}: ctrl_flags1");
    }
    let root_capable = pins.iter().filter(|p| p.2 == CTRL_FLAGS1_ROOT).count();
    eprintln!(
        "parse_control_flags1 corpus pin: checked {} fixtures ({root_capable} root-capable, \
         {} unprivileged)",
        pins.len(),
        pins.len() - root_capable,
    );
    if root_capable == 0 || root_capable == pins.len() {
        eprintln!(
            "parse_control_flags1 corpus pin: only one privilege class is installed, so this \
             run did not hold the root-vs-non-root split apart"
        );
    }
}
