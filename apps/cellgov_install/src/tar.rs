//! USTAR TAR archive parser and extractor.
//!
//! Only regular files are returned. A directory record is dropped, and
//! every other record type is a named refusal rather than a silent
//! skip. Records are padded to 512-byte boundaries.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

use std::io;
use std::path::{Component, Path, PathBuf};

use cellgov_ps3_abi::format::dev_flash::{FLASH_MOUNT, SIBLING_FLASH_MOUNTS};

use crate::field::usize_from_header;

/// Bytes of one header block, and the unit every payload pads to.
const BLOCK: usize = 512;

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
        /// Host path the entry would have resolved to under `vfs_root`.
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
        /// Declared payload size from the header (bytes). Held at the
        /// archive field's own width so an oversized value is reported
        /// rather than truncated into `usize`.
        size: u64,
        /// Total archive size for context.
        archive_size: usize,
    },
    /// A 512-byte block that is neither the terminating all-zero block
    /// nor a USTAR header.
    #[error("tar: block at offset 0x{offset:x} carries no USTAR magic")]
    NotUstarHeader {
        /// Byte offset of the block that failed the magic check.
        offset: usize,
    },
    /// A record's type flag names a record kind this loader does not
    /// model.
    #[error(
        "tar: header at offset 0x{offset:x} ({name:?}) has unsupported file type 0x{filetype:02x}"
    )]
    UnsupportedFileType {
        /// Byte offset of the offending header in the archive.
        offset: usize,
        /// Assembled full name of the entry carrying the type flag.
        name: String,
        /// Raw USTAR type flag byte (header offset 0x9C).
        filetype: u8,
    },
}

/// Summary returned by [`extract_to_disk`].
///
/// `written + skipped + errors.len()` equals the number of entries
/// handed in, so no entry leaves the extractor untallied.
#[derive(Debug, Default)]
pub struct ExtractReport {
    /// Number of entries successfully written to disk.
    pub written: usize,
    /// Entries whose name addressed no file ([`route_entry_path`]
    /// returned `None`), so nothing was written and nothing failed.
    pub skipped: usize,
    /// Per-entry failures, in the order they occurred.
    pub errors: Vec<ExtractError>,
}

/// USTAR prefix field (POSIX.1-1988 / IEEE Std 1003.1). Lies past
/// devmajor (0x149..0x151) and devminor (0x151..0x159).
const PREFIX_FIELD_OFFSET: usize = 0x159;
const PREFIX_FIELD_SIZE: usize = 155;

/// USTAR magic field, sitting just past the 100-byte linkname.
///
/// POSIX ustar (IEEE Std 1003.1) makes this field the mark of a ustar
/// header block. The parser compares only the first five bytes:
///
/// - a POSIX writer follows `ustar` with `\0` plus a two-digit
///   version;
/// - a GNU writer follows it with two spaces.
///
/// Both spell the same layout for every field this loader reads.
const MAGIC_FIELD_OFFSET: usize = 0x101;
const USTAR_MAGIC: &[u8] = b"ustar";

/// Type flag for a regular file. POSIX keeps NUL as the older spelling
/// of the same type, so the parser accepts both.
const TYPE_REGULAR: u8 = b'0';
/// Type flag for a directory record.
const TYPE_DIRECTORY: u8 = b'5';

/// Decode a USTAR octal numeric field, which pads with any mix of NUL
/// and blank at either end.
///
/// A field holding no octal digits at all is a decode failure, not a
/// zero. POSIX spells the size field -- the only field this decodes --
/// as a zero-filled octal number that NUL or blank terminates. It
/// gives no spelling without digits. A zero invented here would turn a
/// malformed record into an empty file the archive never described.
fn octal_to_u64(s: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(s).ok()?;
    let s = s.trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
    u64::from_str_radix(s, 8).ok()
}

/// Parse a USTAR archive into its regular-file entries.
///
/// Directory records carry no payload and are dropped; every other
/// non-regular type is a [`TarParseError::UnsupportedFileType`]
/// refusal. POSIX ustar defines further type flags -- hard and
/// symbolic links, character and block devices, FIFOs, contiguous
/// files -- and GNU writers add long-name and long-link records.
/// Firmware payloads carry none of them. A refusal by name also keeps
/// the parser from swallowing a GNU long-name (`L`) record, which
/// would truncate the next entry's path to the 100-byte name field.
///
/// Zero-byte regular files ARE returned (with empty `data`): PS3
/// firmware ships empty placeholder files the install must reproduce.
/// The first all-zero 512-byte block terminates the archive; anything
/// past it, padding or not, is not read.
pub fn parse(data: &[u8]) -> Result<Vec<TarEntry>, TarParseError> {
    let mut entries = Vec::new();
    let mut rest = data;

    while let Some((header, after_header)) = rest.split_first_chunk::<BLOCK>() {
        let offset = offset_in(data, rest);
        if header.iter().all(|&b| b == 0) {
            break;
        }

        // Without the magic, the fields below are just whatever bytes
        // happen to sit at those offsets -- a name and a size invented
        // out of unrelated data. POSIX defines those offsets for the
        // ustar header block the magic names, and says nothing about a
        // block without one. Refusing every such block, rather than
        // resyncing to the next one, is CellGov's own choice.
        if !header[MAGIC_FIELD_OFFSET..].starts_with(USTAR_MAGIC) {
            return Err(TarParseError::NotUstarHeader { offset });
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

        let declared_size = octal_to_u64(&header[0x7C..0x7C + 12]).ok_or_else(|| {
            TarParseError::UnparseableSize {
                offset,
                name: full_name.clone(),
            }
        })?;
        let filetype = header[0x9C];

        // Bound the payload of EVERY record, not just the ones whose
        // bytes are kept. A record whose payload is skipped still
        // advances the scan by its declared size. An over-long size on
        // such a record would walk past the archive and end the scan
        // with `Ok`, dropping every entry behind it. No published rule
        // covers a size field longer than the archive that holds it,
        // so CellGov refuses it whatever the type flag says.
        let Some(payload) = usize_from_header(declared_size).and_then(|n| after_header.get(..n))
        else {
            return Err(TarParseError::PayloadPastArchive {
                name: full_name,
                offset: offset_in(data, after_header),
                size: declared_size,
                archive_size: data.len(),
            });
        };

        match filetype {
            TYPE_REGULAR | 0 => entries.push(TarEntry {
                name: full_name,
                data: payload.to_vec(),
            }),
            // A directory record carries no payload, and every parent a
            // written file needs is created during extraction, so the
            // record itself is redundant for all but an empty directory.
            TYPE_DIRECTORY => {}
            filetype => {
                return Err(TarParseError::UnsupportedFileType {
                    offset,
                    name: full_name,
                    filetype,
                });
            }
        }

        // A last record whose padding the archive omits ends the scan,
        // as a short trailing block does.
        rest = payload
            .len()
            .checked_next_multiple_of(BLOCK)
            .and_then(|padded| after_header.get(padded..))
            .unwrap_or_default();
    }

    Ok(entries)
}

/// Byte offset of `rest` inside `archive`, of which it is a suffix.
fn offset_in(archive: &[u8], rest: &[u8]) -> usize {
    archive
        .len()
        .checked_sub(rest.len())
        .expect("invariant: the scan only narrows the archive to a suffix of itself")
}

fn is_safe_relative(clean: &str) -> bool {
    Path::new(clean)
        .components()
        .all(|c| !matches!(c, Component::ParentDir))
}

/// The part of `clean` under `mount`, or `None` when `clean` is not
/// under it.
///
/// `mount` is a bare component. The match requires a `/` or the end of
/// the string after it, so `dev_flash2foo/x` names no flash-2 content.
/// An empty result means `clean` addresses the mount root itself
/// (`dev_flash2`, `dev_flash2/`, `dev_flash2//`), which is not a file.
fn under_mount<'a>(clean: &'a str, mount: &str) -> Option<&'a str> {
    let rest = clean.strip_prefix(mount)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix('/').map(|r| r.trim_start_matches('/'))
}

/// VFS-root-relative destination for one archive entry name, or
/// `None` when the name resolves to no file (empty, or a mount root).
///
/// A leading `/` and the `000/` packaging artefact are stripped; a
/// name without a [`SIBLING_FLASH_MOUNTS`] prefix is dev_flash content
/// whether or not it spells `dev_flash/` out.
///
/// The result is relative but not traversal-checked: a caller that
/// joins it onto a real directory must reject `..` itself.
///
/// # Examples
///
/// ```
/// use cellgov_install::tar::route_entry_path;
/// assert_eq!(route_entry_path("000/vsh/module/a.self").as_deref(), Some("dev_flash/vsh/module/a.self"));
/// assert_eq!(route_entry_path("dev_flash2/etc/x.sys").as_deref(), Some("dev_flash2/etc/x.sys"));
/// assert_eq!(route_entry_path("dev_flash2/"), None);
/// assert_eq!(route_entry_path("dev_flash2"), None);
/// ```
pub fn route_entry_path(name: &str) -> Option<String> {
    let clean = name.trim_start_matches('/');
    let clean = clean.strip_prefix("000/").unwrap_or(clean);
    let clean = clean.trim_start_matches('/');
    if clean.is_empty() {
        return None;
    }
    for mount in SIBLING_FLASH_MOUNTS {
        if let Some(inner) = under_mount(clean, mount) {
            if inner.is_empty() {
                return None;
            }
            return Some(format!("{mount}/{inner}"));
        }
    }
    let inner = match under_mount(clean, FLASH_MOUNT) {
        Some("") => return None,
        Some(inner) => inner,
        None => clean,
    };
    Some(format!("{FLASH_MOUNT}/{inner}"))
}

/// Write `entries` under `vfs_root`, routing each name through
/// [`route_entry_path`] so `dev_flash` content lands in
/// `vfs_root/dev_flash/` and each [`SIBLING_FLASH_MOUNTS`] mount lands
/// beside it.
///
/// Path-traversal (`..`) entries are rejected and recorded in the
/// returned report. Per-entry I/O failures are collected rather than
/// short-circuiting; the caller decides whether the report's `errors`
/// vec aborts the install.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "the tallies count entries, so none passes entries.len()"
)]
pub fn extract_to_disk(entries: &[TarEntry], vfs_root: &Path) -> ExtractReport {
    let mut report = ExtractReport::default();
    for entry in entries {
        // Empty `data` is still written (PS3 firmware ships 0-byte
        // placeholder files); only a name that routes to no file skips.
        let Some(routed) = route_entry_path(&entry.name) else {
            report.skipped += 1;
            continue;
        };
        let clean: &str = &routed;
        if !is_safe_relative(clean) {
            report.errors.push(ExtractError::PathTraversal {
                guest_path: entry.name.clone(),
                host_path: vfs_root.join(clean),
            });
            continue;
        }
        let dest: PathBuf = vfs_root.join(clean);
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
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic headers"
)]
#[path = "tests/tar_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic headers"
)]
#[path = "tests/tar_bounds_tests.rs"]
mod bounds_tests;
