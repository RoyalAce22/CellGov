//! USTAR archive parsing: prefix-field path assembly, size decoding, and bounds rejection.

use super::*;
use crate::scratch_dir::scratch;

/// Build a single USTAR 512-byte header.
fn ustar_header(name: &str, prefix: &str, size: usize, typeflag: u8) -> [u8; 512] {
    assert!(name.len() < 100, "name must be < 100 bytes");
    assert!(prefix.len() < 155, "prefix must be < 155 bytes");
    let mut h = [0u8; 512];
    h[0..name.len()].copy_from_slice(name.as_bytes());
    let size_oct = format!("{size:o}");
    h[0x7C..0x7C + size_oct.len()].copy_from_slice(size_oct.as_bytes());
    h[0x9C] = typeflag;
    h[0x101..0x106].copy_from_slice(b"ustar");
    h[PREFIX_FIELD_OFFSET..PREFIX_FIELD_OFFSET + prefix.len()].copy_from_slice(prefix.as_bytes());
    h
}

#[test]
fn parse_empty_returns_empty() {
    assert!(parse(&[]).unwrap().is_empty());
}

#[test]
fn parse_all_zeros_returns_empty() {
    assert!(parse(&[0u8; 1024]).unwrap().is_empty());
}

#[test]
fn parse_long_path_uses_prefix_field() {
    let prefix = "a_long_directory_prefix_to_force_the_split";
    let name = "the_actual_filename.bin";
    let body = b"hello";
    let mut data = Vec::new();
    data.extend_from_slice(&ustar_header(name, prefix, body.len(), b'0'));
    let mut block = [0u8; 512];
    block[..body.len()].copy_from_slice(body);
    data.extend_from_slice(&block);

    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, format!("{prefix}/{name}"));
    assert_eq!(entries[0].data, body);
}

/// Populates devmajor/devminor (0x149..0x159) with non-zero ASCII
/// octals; a reader off-by-8 would splice them into the path.
#[test]
fn parse_ignores_devmajor_devminor_when_assembling_path() {
    let name = "f.bin";
    let body = b"x";
    let mut header = ustar_header(name, "", body.len(), b'0');
    for b in &mut header[0x149..0x159] {
        *b = b'7';
    }
    let mut data = Vec::new();
    data.extend_from_slice(&header);
    let mut block = [0u8; 512];
    block[..body.len()].copy_from_slice(body);
    data.extend_from_slice(&block);

    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, name);
}

#[test]
fn parse_rejects_unparseable_size() {
    let mut header = ustar_header("f.bin", "", 0, b'0');
    for b in &mut header[0x7C..0x7C + 12] {
        *b = b'?';
    }
    let err = parse(&header).unwrap_err();
    assert!(matches!(err, TarParseError::UnparseableSize { .. }));
}

/// A size field padded out to nothing is a malformed record, not a
/// zero-length file the archive asked for.
#[test]
fn parse_rejects_a_size_field_holding_no_octal_digits() {
    let mut header = ustar_header("f.bin", "", 0, TYPE_REGULAR);
    header[0x7C..0x7C + 12].fill(b' ');
    let err = parse(&header).unwrap_err();
    assert!(
        matches!(err, TarParseError::UnparseableSize { .. }),
        "{err}"
    );
}

#[test]
fn a_size_field_padded_with_nul_and_blank_at_both_ends_still_decodes() {
    let mut header = ustar_header("f.bin", "", 0, TYPE_REGULAR);
    let mut field = [0u8; 12];
    field[0] = b' ';
    field[2..11].copy_from_slice(b"000000144");
    header[0x7C..0x7C + 12].copy_from_slice(&field);
    let mut data = Vec::new();
    data.extend_from_slice(&header);
    data.extend_from_slice(&[0u8; 512]);

    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data.len(), 0o144);
}

#[test]
fn parse_rejects_payload_past_eof() {
    let header = ustar_header("f.bin", "", 100, b'0');
    let err = parse(&header).unwrap_err();
    assert!(matches!(err, TarParseError::PayloadPastArchive { .. }));
}

#[test]
fn parse_refuses_a_block_that_carries_no_ustar_magic() {
    let mut header = ustar_header("f.bin", "", 0, TYPE_REGULAR);
    header[MAGIC_FIELD_OFFSET] = b'x';
    let err = parse(&header).unwrap_err();
    assert!(
        matches!(err, TarParseError::NotUstarHeader { offset: 0 }),
        "{err}"
    );
}

#[test]
fn parse_walks_past_a_directory_record_to_the_files_behind_it() {
    let mut data = Vec::new();
    data.extend_from_slice(&ustar_header("dev_flash/vsh/", "", 0, TYPE_DIRECTORY));
    data.extend_from_slice(&ustar_header("dev_flash/vsh/a.self", "", 1, TYPE_REGULAR));
    let mut block = [0u8; 512];
    block[0] = b'A';
    data.extend_from_slice(&block);

    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "dev_flash/vsh/a.self");
    assert_eq!(entries[0].data, b"A");
}

/// The scan advances by a skipped record's declared size just as it
/// does for a kept one, so an over-long size on a directory record
/// would otherwise end the archive early and return `Ok` having
/// dropped everything behind it.
#[test]
fn parse_bounds_a_skipped_records_payload_against_the_archive() {
    let mut data = Vec::new();
    data.extend_from_slice(&ustar_header("bogus_dir/", "", 0x10_0000, TYPE_DIRECTORY));
    data.extend_from_slice(&ustar_header("after.bin", "", 0, TYPE_REGULAR));

    let err = parse(&data).unwrap_err();
    assert!(
        matches!(err, TarParseError::PayloadPastArchive { .. }),
        "{err}"
    );
}

#[test]
fn parse_refuses_a_gnu_long_name_record_rather_than_truncating_the_next_path() {
    let mut data = Vec::new();
    data.extend_from_slice(&ustar_header("././@LongLink", "", 0, b'L'));
    data.extend_from_slice(&ustar_header("short.bin", "", 0, TYPE_REGULAR));

    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            TarParseError::UnsupportedFileType {
                offset: 0,
                filetype: b'L',
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn parse_refuses_a_symlink_record() {
    let header = ustar_header("link", "", 0, b'2');
    let err = parse(&header).unwrap_err();
    assert!(
        matches!(
            err,
            TarParseError::UnsupportedFileType { filetype: b'2', .. }
        ),
        "{err}"
    );
}

#[test]
fn extract_strips_pup_prefixes() {
    let dir = scratch();

    let entries = vec![
        TarEntry {
            name: "dev_flash/sys/external/liblv2.sprx".into(),
            data: b"L".to_vec(),
        },
        TarEntry {
            name: "000/sys/internal/x.sprx".into(),
            data: b"X".to_vec(),
        },
        TarEntry {
            name: "dev_flash2/keep/y.bin".into(),
            data: b"Y".to_vec(),
        },
        TarEntry {
            name: "dev_flash3/keep/z.bin".into(),
            data: b"Z".to_vec(),
        },
        TarEntry {
            name: "vsh/module/w.self".into(),
            data: b"W".to_vec(),
        },
    ];

    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 5);
    assert!(report.errors.is_empty());

    // `dir` is the VFS root: dev_flash content lands under dev_flash/,
    // whether the PUP entry carried the prefix (liblv2), carried the
    // `000/` packaging artefact instead (x.sprx), or carried neither
    // (w.self).
    assert!(dir.join("dev_flash/sys/external/liblv2.sprx").is_file());
    assert!(dir.join("dev_flash/sys/internal/x.sprx").is_file());
    assert!(dir.join("dev_flash/vsh/module/w.self").is_file());
    // dev_flash2 and dev_flash3 are sibling mounts, not content inside
    // dev_flash.
    assert!(dir.join("dev_flash2/keep/y.bin").is_file());
    assert!(dir.join("dev_flash3/keep/z.bin").is_file());
    assert!(!dir.join("dev_flash/dev_flash2").exists());
    assert!(!dir.join("dev_flash/dev_flash3").exists());
}

#[test]
fn route_strips_leading_slashes_and_the_packaging_prefix() {
    assert_eq!(
        route_entry_path("/000/vsh/module/a.self").as_deref(),
        Some("dev_flash/vsh/module/a.self")
    );
    assert_eq!(
        route_entry_path("//dev_flash//sys/external/b.sprx").as_deref(),
        Some("dev_flash/sys/external/b.sprx")
    );
    assert_eq!(
        route_entry_path("000/dev_flash2/etc/x.sys").as_deref(),
        Some("dev_flash2/etc/x.sys")
    );
    // Only one `000/` layer is packaging; a second is content.
    assert_eq!(
        route_entry_path("000/000/x").as_deref(),
        Some("dev_flash/000/x")
    );
    // Stripping `000/` can itself expose a leading separator, so the
    // sibling match has to run on the re-trimmed name.
    assert_eq!(
        route_entry_path("/000//dev_flash2/x.sys").as_deref(),
        Some("dev_flash2/x.sys")
    );
    assert_eq!(
        route_entry_path("000//000/x").as_deref(),
        Some("dev_flash/000/x")
    );
    assert_eq!(route_entry_path("///x").as_deref(), Some("dev_flash/x"));
}

#[test]
fn route_returns_none_for_a_name_that_addresses_no_file() {
    for name in ["", "/", "000/", "dev_flash/", "dev_flash2/", "dev_flash3//"] {
        assert_eq!(route_entry_path(name), None, "{name:?}");
    }
}

#[test]
fn a_mount_root_addresses_no_file_with_or_without_its_trailing_slash() {
    // `dev_flash2/` and `dev_flash2` name the same thing -- the mount
    // directory.
    for name in [
        "dev_flash",
        "dev_flash2",
        "dev_flash3",
        "/000/dev_flash2",
        "///dev_flash//",
    ] {
        assert_eq!(route_entry_path(name), None, "{name:?}");
    }
}

#[test]
fn sibling_mount_match_needs_a_whole_path_component() {
    // A near-miss is ordinary dev_flash content and keeps its whole
    // name, rather than being split at the `dev_flash2` prefix.
    for (name, expected) in [
        ("dev_flash2foo/x", "dev_flash/dev_flash2foo/x"),
        ("dev_flash20/x", "dev_flash/dev_flash20/x"),
        ("dev_flash2x", "dev_flash/dev_flash2x"),
    ] {
        assert_eq!(
            route_entry_path(name).as_deref(),
            Some(expected),
            "{name:?}"
        );
    }
}

#[test]
fn a_sibling_mount_entry_keeps_one_separator_after_its_prefix() {
    assert_eq!(
        route_entry_path("dev_flash2//etc//x.sys").as_deref(),
        Some("dev_flash2/etc//x.sys")
    );
}

#[test]
fn extract_rejects_traversal_out_of_a_sibling_mount() {
    let dir = scratch();
    let entries = vec![TarEntry {
        name: "dev_flash2/../../escape.bin".into(),
        data: b"nope".to_vec(),
    }];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 0);
    assert_eq!(report.errors.len(), 1);
    assert!(matches!(
        report.errors[0],
        ExtractError::PathTraversal { .. }
    ));
    assert_eq!(
        std::fs::read_dir(&*dir).expect("read_dir").count(),
        0,
        "a refused entry leaves the VFS root untouched"
    );
}

#[test]
fn extract_rejects_traversal_that_would_normalize_back_inside() {
    let dir = scratch();
    let entries = vec![TarEntry {
        name: "dev_flash/../dev_flash2/x.bin".into(),
        data: b"nope".to_vec(),
    }];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 0);
    assert_eq!(report.errors.len(), 1);
    assert!(matches!(
        report.errors[0],
        ExtractError::PathTraversal { .. }
    ));
    // The normalized destination is inside the root, so a traversal
    // check that only compared the final path would have written it.
    assert!(!dir.join("dev_flash2/x.bin").exists());
    assert_eq!(
        std::fs::read_dir(&*dir).expect("read_dir").count(),
        0,
        "a refused entry leaves the VFS root untouched"
    );
}

#[test]
fn extract_skips_a_bare_sibling_mount_root_rather_than_shadowing_it() {
    let dir = scratch();
    let entries = vec![
        TarEntry {
            name: "dev_flash2/".into(),
            data: b"shadow".to_vec(),
        },
        TarEntry {
            name: "dev_flash2/etc/x.sys".into(),
            data: b"X".to_vec(),
        },
    ];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 1);
    assert!(report.errors.is_empty());
    assert!(dir.join("dev_flash2/etc/x.sys").is_file());
    assert!(dir.join("dev_flash2").is_dir());
}

#[test]
fn extract_tallies_an_entry_that_addresses_no_file_instead_of_dropping_it() {
    let dir = scratch();
    let entries = vec![
        TarEntry {
            name: "dev_flash2/".into(),
            data: b"shadow".to_vec(),
        },
        TarEntry {
            name: String::new(),
            data: b"nameless".to_vec(),
        },
        TarEntry {
            name: "dev_flash/keep.bin".into(),
            data: b"K".to_vec(),
        },
    ];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 1);
    assert_eq!(report.skipped, 2);
    assert!(report.errors.is_empty());
    // Every entry handed in is accounted for by exactly one tally.
    assert_eq!(
        report.written + report.skipped + report.errors.len(),
        entries.len()
    );
}

#[test]
fn every_entry_lands_in_exactly_one_extract_tally_including_the_error_arm() {
    let dir = scratch();
    let entries = vec![
        TarEntry {
            name: "dev_flash/keep.bin".into(),
            data: b"K".to_vec(),
        },
        TarEntry {
            name: "dev_flash/".into(),
            data: Vec::new(),
        },
        TarEntry {
            name: "../escape.bin".into(),
            data: b"nope".to_vec(),
        },
    ];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.written + report.skipped + report.errors.len(),
        entries.len()
    );
}

#[test]
fn extract_rejects_path_traversal() {
    let dir = scratch();

    let entries = vec![TarEntry {
        name: "../escape.bin".into(),
        data: b"nope".to_vec(),
    }];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 0);
    assert_eq!(report.errors.len(), 1);
    assert!(matches!(
        report.errors[0],
        ExtractError::PathTraversal { .. }
    ));
}

#[test]
fn parse_minimal_tar_entry() {
    let header = ustar_header("hello.txt", "", 5, b'0');
    let mut data = Vec::new();
    data.extend_from_slice(&header);
    let mut payload = [0u8; 512];
    payload[..5].copy_from_slice(b"hello");
    data.extend_from_slice(&payload);

    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "hello.txt");
    assert_eq!(entries[0].data, b"hello");
}

#[test]
fn parse_keeps_zero_byte_regular_file() {
    // PS3 firmware ships empty placeholder files (e.g. vsh dummy.txt).
    let header = ustar_header("empty.txt", "", 0, b'0');
    let entries = parse(&header).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "empty.txt");
    assert!(entries[0].data.is_empty());
}

#[test]
fn extract_writes_zero_byte_file() {
    let dir = scratch();

    let entries = vec![TarEntry {
        name: "dev_flash/vsh/resource/silk/lib/Plugins/dummy.txt".into(),
        data: Vec::new(),
    }];
    let report = extract_to_disk(&entries, &dir);
    assert_eq!(report.written, 1);
    assert!(report.errors.is_empty());
    let dest = dir.join("dev_flash/vsh/resource/silk/lib/Plugins/dummy.txt");
    assert!(dest.is_file());
    assert_eq!(std::fs::metadata(&dest).unwrap().len(), 0);
}
