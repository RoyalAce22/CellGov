//! USTAR archive parsing: prefix-field path assembly, size decoding, and bounds rejection.

use super::*;

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

#[test]
fn parse_rejects_payload_past_eof() {
    let header = ustar_header("f.bin", "", 100, b'0');
    let err = parse(&header).unwrap_err();
    assert!(matches!(err, TarParseError::PayloadPastArchive { .. }));
}

#[test]
fn extract_strips_pup_prefixes() {
    let dir = std::env::temp_dir().join("cellgov_tar_strip_31c1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

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

    assert!(dir.join("sys/external/liblv2.sprx").is_file());
    assert!(dir.join("sys/internal/x.sprx").is_file());
    assert!(dir.join("dev_flash2/keep/y.bin").is_file());
    assert!(dir.join("dev_flash3/keep/z.bin").is_file());
    assert!(dir.join("vsh/module/w.self").is_file());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extract_rejects_path_traversal() {
    let dir = std::env::temp_dir().join("cellgov_tar_traverse_31c1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

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

    std::fs::remove_dir_all(&dir).unwrap();
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
