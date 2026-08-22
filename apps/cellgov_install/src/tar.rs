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
        /// Declared payload size from the header (bytes).
        size: usize,
        /// Total archive size for context.
        archive_size: usize,
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
/// silently skipped. Zero-byte regular files ARE returned (with empty
/// `data`): PS3 firmware ships empty placeholder files the install
/// must reproduce. A pair of all-zero 512-byte blocks terminates the
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

        if filetype == b'0' || filetype == 0 {
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

fn is_safe_relative(clean: &str) -> bool {
    Path::new(clean)
        .components()
        .all(|c| !matches!(c, Component::ParentDir))
}

/// Mounts a PUP carries alongside `dev_flash`, named by their own
/// prefix; they are siblings of `dev_flash` on the console, so they
/// keep their prefix and land beside it.
///
/// LV2 publishes exactly `/dev_flash`, `/dev_flash2` and
/// `/dev_flash3` as flash mount points, and `/dev_flash` is itself
/// flash 1, so the set is closed at two. RPCS3 mirrors the same three
/// in `Emu/System.cpp` `Emulator::Init` and `Emu/Cell/lv2/sys_fs.cpp`
/// `g_mp_sys_dev_flash{,2,3}`.
pub const SIBLING_MOUNTS: [&str; 2] = ["dev_flash2/", "dev_flash3/"];

/// The flash-1 mount every name without a [`SIBLING_MOUNTS`] prefix
/// belongs to, whether or not it spells the prefix out.
const DEV_FLASH_MOUNT: &str = "dev_flash/";

/// Whether `clean` addresses `mount` itself rather than a file under
/// it -- `dev_flash2`, `dev_flash2/`, `dev_flash2//` and so on.
///
/// A tar entry naming a mount point addresses the mount directory:
/// RPCS3 resolves the name through the mount table and then fails the
/// write against the directory that is already there
/// (`Loader/TAR.cpp` `tar_object::extract`, mounts registered in
/// `Emu/System.cpp` `Emulator::Init`).
fn addresses_mount_root(clean: &str, mount: &str) -> bool {
    let bare = mount.trim_end_matches('/');
    clean == bare
        || clean
            .strip_prefix(mount)
            .is_some_and(|rest| rest.trim_start_matches('/').is_empty())
}

/// VFS-root-relative destination for one archive entry name, or
/// `None` when the name resolves to no file (empty, or a mount root).
///
/// A leading `/` and the `000/` packaging artefact are stripped; each
/// [`SIBLING_MOUNTS`] prefix is kept, and everything else is dev_flash
/// content, so a `dev_flash/`-prefixed name and a prefixless one
/// resolve to the same place.
///
/// The result always starts with a mount component and is therefore
/// relative, but it is NOT traversal-checked -- a caller that joins it
/// onto a real directory must reject `..` itself.
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
    for mount in SIBLING_MOUNTS {
        if addresses_mount_root(clean, mount) {
            return None;
        }
        if let Some(rest) = clean.strip_prefix(mount) {
            return Some(format!("{mount}{}", rest.trim_start_matches('/')));
        }
    }
    if addresses_mount_root(clean, DEV_FLASH_MOUNT) {
        return None;
    }
    let inner = clean
        .strip_prefix(DEV_FLASH_MOUNT)
        .unwrap_or(clean)
        .trim_start_matches('/');
    if inner.is_empty() {
        return None;
    }
    Some(format!("{DEV_FLASH_MOUNT}{inner}"))
}

/// Write `entries` under `vfs_root`, routing each name through
/// [`route_entry_path`] so `dev_flash` content lands in
/// `vfs_root/dev_flash/` and each [`SIBLING_MOUNTS`] mount lands
/// beside it.
///
/// Path-traversal (`..`) entries are rejected and recorded in the
/// returned report. Per-entry I/O failures are collected rather than
/// short-circuiting; the caller decides whether the report's `errors`
/// vec aborts the install.
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
#[path = "tests/tar_tests.rs"]
mod tests;
