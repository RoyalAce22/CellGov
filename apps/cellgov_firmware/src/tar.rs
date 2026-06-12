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
#[path = "tests/tar_tests.rs"]
mod tests;
