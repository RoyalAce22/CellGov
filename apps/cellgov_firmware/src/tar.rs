//! USTAR TAR archive parser and extractor.
//!
//! Only regular files are returned; symlinks and other non-regular types are
//! skipped. Records are padded to 512-byte boundaries.

use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct TarEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Per-file failure surfaced by [`extract_to_disk`].
#[derive(Debug)]
pub struct ExtractError {
    pub guest_path: String,
    pub host_path: PathBuf,
    pub kind: ExtractErrorKind,
}

#[derive(Debug)]
pub enum ExtractErrorKind {
    PathTraversal,
    CreateDir(io::Error),
    Write(io::Error),
}

/// Summary returned by [`extract_to_disk`]. `errors` is non-empty when
/// one or more entries failed; the caller decides whether that aborts
/// the install or is logged and tolerated.
#[derive(Debug, Default)]
pub struct ExtractReport {
    pub written: usize,
    pub errors: Vec<ExtractError>,
}

/// USTAR prefix field (POSIX.1-1988 / IEEE Std 1003.1). Lies past
/// devmajor (0x149..0x151) and devminor (0x151..0x159).
const PREFIX_FIELD_OFFSET: usize = 0x159;
const PREFIX_FIELD_SIZE: usize = 155;

fn octal_to_u64(s: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(s).ok()?;
    let s = s.trim_end_matches('\0').trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

pub fn parse(data: &[u8]) -> Result<Vec<TarEntry>, String> {
    let mut entries = Vec::new();
    let mut offset = 0usize;

    while offset + 512 <= data.len() {
        let header = &data[offset..offset + 512];

        if header.iter().all(|&b| b == 0) {
            break;
        }

        let name_raw = &header[0..100];
        let name_end = name_raw.iter().position(|&b| b == 0).unwrap_or(100);
        let name_str = std::str::from_utf8(&name_raw[..name_end])
            .map_err(|e| format!("tar: header at offset 0x{offset:x} has non-UTF-8 name: {e}"))?;

        let prefix_raw = &header[PREFIX_FIELD_OFFSET..PREFIX_FIELD_OFFSET + PREFIX_FIELD_SIZE];
        let prefix_end = prefix_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(PREFIX_FIELD_SIZE);
        let prefix_str = std::str::from_utf8(&prefix_raw[..prefix_end])
            .map_err(|e| format!("tar: header at offset 0x{offset:x} has non-UTF-8 prefix: {e}"))?;

        let full_name = if prefix_str.is_empty() {
            name_str.to_string()
        } else {
            format!("{prefix_str}/{name_str}")
        };

        let size = octal_to_u64(&header[0x7C..0x7C + 12]).ok_or_else(|| {
            format!("tar: header at offset 0x{offset:x} ({full_name:?}) has unparseable size field")
        })? as usize;
        let filetype = header[0x9C];

        offset += 512;

        if (filetype == b'0' || filetype == 0) && size > 0 {
            if offset + size > data.len() {
                return Err(format!(
                    "tar: entry {full_name:?} payload extends past archive (offset 0x{offset:x}, size 0x{size:x}, archive 0x{:x})",
                    data.len()
                ));
            }
            entries.push(TarEntry {
                name: full_name,
                data: data[offset..offset + size].to_vec(),
            });
        }

        let aligned = (size + 511) & !511;
        offset += aligned;
    }

    Ok(entries)
}

/// Reject any cleaned path containing a `ParentDir` component.
fn is_safe_relative(clean: &str) -> bool {
    Path::new(clean)
        .components()
        .all(|c| !matches!(c, Component::ParentDir))
}

pub fn extract_to_disk(entries: &[TarEntry], base: &Path) -> ExtractReport {
    let mut report = ExtractReport::default();
    for entry in entries {
        if entry.name.is_empty() || entry.data.is_empty() {
            continue;
        }
        let clean = entry.name.trim_start_matches('/');
        // `base` is the dev_flash VFS root, so "dev_flash/" and the
        // "000/" packaging artefact strip; "dev_flash2/" and
        // "dev_flash3/" are sibling mounts and stay intact.
        let clean = clean.strip_prefix("000/").unwrap_or(clean);
        let clean = clean.strip_prefix("dev_flash/").unwrap_or(clean);
        if clean.is_empty() {
            continue;
        }
        if !is_safe_relative(clean) {
            report.errors.push(ExtractError {
                guest_path: entry.name.clone(),
                host_path: base.join(clean),
                kind: ExtractErrorKind::PathTraversal,
            });
            continue;
        }
        let dest: PathBuf = base.join(clean);
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                report.errors.push(ExtractError {
                    guest_path: entry.name.clone(),
                    host_path: dest.clone(),
                    kind: ExtractErrorKind::CreateDir(e),
                });
                continue;
            }
        }
        match std::fs::write(&dest, &entry.data) {
            Ok(()) => report.written += 1,
            Err(e) => report.errors.push(ExtractError {
                guest_path: entry.name.clone(),
                host_path: dest,
                kind: ExtractErrorKind::Write(e),
            }),
        }
    }
    report
}

#[cfg(test)]
mod tests {
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
        h[PREFIX_FIELD_OFFSET..PREFIX_FIELD_OFFSET + prefix.len()]
            .copy_from_slice(prefix.as_bytes());
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
        assert!(err.contains("unparseable size"));
    }

    #[test]
    fn parse_rejects_payload_past_eof() {
        let header = ustar_header("f.bin", "", 100, b'0');
        let err = parse(&header).unwrap_err();
        assert!(err.contains("extends past archive"));
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
            report.errors[0].kind,
            ExtractErrorKind::PathTraversal
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
}
