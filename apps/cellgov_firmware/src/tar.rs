//! USTAR TAR archive parser and extractor.
//!
//! Only regular files are returned; symlinks and other non-regular types are
//! skipped. Records are padded to 512-byte boundaries.

use std::io;
use std::path::{Component, Path, PathBuf};

/// One regular file extracted from a USTAR archive.
#[derive(Debug)]
pub struct TarEntry {
    /// Full archive-relative path (prefix + `/` + name when the USTAR
    /// header used the prefix field; bare name otherwise).
    pub name: String,
    /// Raw file payload, unpadded.
    pub data: Vec<u8>,
}

/// Per-file failure surfaced by [`extract_to_disk`].
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// Cleaned guest path contained a `..` component.
    #[error("path traversal: {guest_path} -> {}", host_path.display())]
    PathTraversal {
        /// Original archive-relative path as recorded in the tar header.
        guest_path: String,
        /// Host path the entry would have resolved to under `base`.
        host_path: PathBuf,
    },
    /// `create_dir_all` on the destination's parent failed.
    #[error("create_dir_all for {guest_path} -> {}: {source}", host_path.display())]
    CreateDir {
        /// Original archive-relative path.
        guest_path: String,
        /// Host destination whose parent could not be created.
        host_path: PathBuf,
        /// Underlying filesystem error from `std::fs::create_dir_all`.
        #[source]
        source: io::Error,
    },
    /// `std::fs::write` on the destination failed.
    #[error("write {guest_path} -> {}: {source}", host_path.display())]
    Write {
        /// Original archive-relative path.
        guest_path: String,
        /// Host destination whose write failed.
        host_path: PathBuf,
        /// Underlying filesystem error from `std::fs::write`.
        #[source]
        source: io::Error,
    },
}

/// Why USTAR `parse` rejected the archive.
#[derive(Debug, thiserror::Error)]
pub enum TarParseError {
    /// A header's name field is not valid UTF-8.
    #[error("tar: header at offset 0x{offset:x} has non-UTF-8 name: {source}")]
    NameNotUtf8 {
        /// Byte offset of the offending 512-byte header in the archive.
        offset: usize,
        /// UTF-8 decode error from the name field (header bytes 0..100).
        #[source]
        source: std::str::Utf8Error,
    },
    /// A header's prefix field is not valid UTF-8.
    #[error("tar: header at offset 0x{offset:x} has non-UTF-8 prefix: {source}")]
    PrefixNotUtf8 {
        /// Byte offset of the offending 512-byte header in the archive.
        offset: usize,
        /// UTF-8 decode error from the USTAR prefix field
        /// (header bytes 0x159..0x1F4).
        #[source]
        source: std::str::Utf8Error,
    },
    /// The size field is not a valid octal string.
    #[error("tar: header at offset 0x{offset:x} ({name:?}) has unparseable size field")]
    UnparseableSize {
        /// Byte offset of the offending header in the archive.
        offset: usize,
        /// Assembled full name of the entry whose size field failed to parse.
        name: String,
    },
    /// The entry's declared payload extends past the archive.
    #[error("tar: entry {name:?} payload extends past archive (offset 0x{offset:x}, size 0x{size:x}, archive 0x{archive_size:x})")]
    PayloadPastArchive {
        /// Assembled full name of the over-long entry.
        name: String,
        /// Byte offset where the payload was expected to start.
        offset: usize,
        /// Declared payload size from the header (bytes).
        size: usize,
        /// Total archive size for context.
        archive_size: usize,
    },
}

/// Summary returned by [`extract_to_disk`]. `errors` is non-empty when
/// one or more entries failed; the caller decides whether that aborts
/// the install or is logged and tolerated.
#[derive(Debug, Default)]
pub struct ExtractReport {
    /// Number of entries successfully written to disk.
    pub written: usize,
    /// Per-entry failures, in the order they occurred.
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

/// Parse a USTAR archive into its regular-file entries.
///
/// Non-regular records (symlinks, directories, device nodes, etc.) are
/// silently skipped. A pair of all-zero 512-byte blocks terminates the
/// archive; trailing padding past that is ignored.
pub fn parse(data: &[u8]) -> Result<Vec<TarEntry>, TarParseError> {
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
            .map_err(|source| TarParseError::NameNotUtf8 { offset, source })?;

        let prefix_raw = &header[PREFIX_FIELD_OFFSET..PREFIX_FIELD_OFFSET + PREFIX_FIELD_SIZE];
        let prefix_end = prefix_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(PREFIX_FIELD_SIZE);
        let prefix_str = std::str::from_utf8(&prefix_raw[..prefix_end])
            .map_err(|source| TarParseError::PrefixNotUtf8 { offset, source })?;

        let full_name = if prefix_str.is_empty() {
            name_str.to_string()
        } else {
            format!("{prefix_str}/{name_str}")
        };

        let size = octal_to_u64(&header[0x7C..0x7C + 12]).ok_or_else(|| {
            TarParseError::UnparseableSize {
                offset,
                name: full_name.clone(),
            }
        })? as usize;
        let filetype = header[0x9C];

        offset += 512;

        if (filetype == b'0' || filetype == 0) && size > 0 {
            if offset + size > data.len() {
                return Err(TarParseError::PayloadPastArchive {
                    name: full_name,
                    offset,
                    size,
                    archive_size: data.len(),
                });
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

/// Write `entries` under `base`, stripping PUP packaging prefixes
/// (`000/`, `dev_flash/`) so `base` ends up as the dev_flash VFS root.
/// Path-traversal (`..`) entries are rejected and recorded in the
/// returned report. Per-entry I/O failures are collected rather than
/// short-circuiting; the caller decides whether the report's `errors`
/// vec aborts the install.
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
            report.errors.push(ExtractError::PathTraversal {
                guest_path: entry.name.clone(),
                host_path: base.join(clean),
            });
            continue;
        }
        let dest: PathBuf = base.join(clean);
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                report.errors.push(ExtractError::CreateDir {
                    guest_path: entry.name.clone(),
                    host_path: dest.clone(),
                    source: e,
                });
                continue;
            }
        }
        match std::fs::write(&dest, &entry.data) {
            Ok(()) => report.written += 1,
            Err(e) => report.errors.push(ExtractError::Write {
                guest_path: entry.name.clone(),
                host_path: dest,
                source: e,
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
}
