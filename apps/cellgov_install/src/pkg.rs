//! Retail PS3 `.pkg` container parsing and content extraction.
//!
//! A finalized retail package is a big-endian header, an AES-128-CTR
//! encrypted item-record table, and the CTR-encrypted item data. The
//! key is the published retail `PKG_AES_KEY` used directly; the
//! per-block CTR counter is the header `klicensee` nonce plus the
//! 16-byte block index (mirroring RPCS3's `unpkg.cpp`).
//!
//! This module is a pure function from package bytes to extracted
//! files. It never touches the filesystem; the install orchestration
//! ([`crate::game_install`]) stages and commits the returned tree. The
//! EBOOT comes out still SCE/NPDRM-wrapped -- extraction is not
//! executable decryption.

use aes::cipher::{BlockEncrypt, KeyInit};
use cellgov_ps3_abi::sce::PKG_AES_KEY;

/// Retail release type (`pkg_type == 0x8000`).
const PKG_TYPE_RELEASE: u16 = 0x8000;
/// Debug release type (`pkg_type == 0x0000`), rejected.
const PKG_TYPE_DEBUG: u16 = 0x0000;
/// PS3 platform tag (`pkg_platform == 0x0001`).
const PKG_PLATFORM_PS3: u16 = 0x0001;
/// Minimum header bytes that must be present to read every field
/// through the 16-byte `klicensee` at 0x70.
const PKG_HEADER_MIN: usize = 0x80;
/// One item record (`PKGEntry`) is 32 bytes.
const ENTRY_LEN: usize = 0x20;
/// Item-name field cap, matching `PKG_MAX_FILENAME_SIZE`.
const MAX_NAME_LEN: u32 = 256;

/// Low byte of `PKGEntry::type`: a directory.
const ENTRY_KIND_FOLDER: u8 = 4;
/// Low byte of `PKGEntry::type`: alternate directory tag (`0x12`).
const ENTRY_KIND_FOLDER_ALT: u8 = 0x12;
/// Low byte of `PKGEntry::type`: NPDRM-EDAT (out of scope, skipped).
const ENTRY_KIND_EDAT: u8 = 2;
/// Low byte of `PKGEntry::type`: SDAT (out of scope, skipped).
const ENTRY_KIND_SDAT: u8 = 9;

/// Parsed retail PKG header. Only the fields the installer consumes
/// are retained; metadata packets are not parsed here.
#[derive(Debug, Clone)]
pub struct PkgHeader {
    /// Release type; only `PKG_TYPE_RELEASE` reaches a caller.
    pub pkg_type: u16,
    /// Platform tag; only `PKG_PLATFORM_PS3` reaches a caller.
    pub pkg_platform: u16,
    /// Number of item records in the encrypted entry table.
    pub file_count: u32,
    /// Total package size in bytes, per the header.
    pub pkg_size: u64,
    /// File offset where the encrypted data region begins.
    pub data_offset: u64,
    /// Encrypted data region length in bytes.
    pub data_size: u64,
    /// Full content id from the 48-byte header field, NUL/space-
    /// trimmed. Despite the field being named `title_id` in the on-disk struct, it
    /// carries the full content id; the 9-char title-id is embedded at
    /// byte 7 (`UP9000-` prefix + title-id), which RPCS3 reads as the
    /// install directory.
    pub content_id: String,
    /// 16-byte CTR nonce.
    pub klicensee: [u8; 16],
}

/// Whether an extracted item is a regular file or a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgEntryKind {
    /// A regular file; [`PkgFile::data`] holds its verbatim bytes.
    File,
    /// A directory; [`PkgFile::data`] is empty.
    Directory,
}

/// One extracted package item: its package-relative path and, for
/// files, the decrypted bytes exactly as they should land on disk.
#[derive(Debug, Clone)]
pub struct PkgFile {
    /// Package-relative path, `/`-separated (e.g. `USRDIR/EBOOT.BIN`).
    pub name: String,
    /// File or directory.
    pub kind: PkgEntryKind,
    /// Verbatim bytes for a file; empty for a directory.
    pub data: Vec<u8>,
}

/// A fully parsed and decrypted package: its header plus every
/// in-scope item, in container declaration order.
#[derive(Debug, Clone)]
pub struct PkgArchive {
    /// Parsed header.
    pub header: PkgHeader,
    /// Extracted items (EDAT/SDAT skipped), in record order.
    pub files: Vec<PkgFile>,
}

/// Why parsing or extracting a retail PKG failed. Every variant is
/// rejected before any byte is written to disk.
#[derive(Debug, thiserror::Error)]
pub enum PkgError {
    /// Input is shorter than the fixed header.
    #[error("PKG too small for header (got {len} bytes, need {min})", min = PKG_HEADER_MIN)]
    TooSmall {
        /// Observed input length.
        len: usize,
    },
    /// Magic was not `\x7FPKG`; carries the first 4 observed bytes.
    #[error("bad PKG magic: {:02x}{:02x}{:02x}{:02x}", _0[0], _0[1], _0[2], _0[3])]
    BadMagic([u8; 4]),
    /// Debug package (`pkg_type == 0x0000`); only retail is supported.
    #[error("debug PKG (pkg_type 0x0000) is not supported; retail only")]
    DebugPackage,
    /// Release type was neither retail nor debug.
    #[error("unknown PKG release type 0x{0:04x}")]
    UnknownReleaseType(u16),
    /// Platform tag was not PS3.
    #[error("non-PS3 PKG platform 0x{0:04x}")]
    NonPs3Platform(u16),
    /// `file_count` is implausibly large for the entry table.
    #[error("PKG file_count 0x{0:x} is too large")]
    FileCountTooLarge(u32),
    /// `pkg_size` exceeds the input length: truncated or multi-part.
    #[error("PKG size mismatch: pkg_size 0x{pkg_size:x} exceeds file length 0x{len:x} (multi-part PKGs are not supported)")]
    SizeMismatch {
        /// `pkg_size` from the header.
        pkg_size: u64,
        /// Actual input length.
        len: usize,
    },
    /// `data_offset + data_size` escapes the package.
    #[error(
        "PKG data region [0x{data_offset:x}, +0x{data_size:x}) escapes pkg_size 0x{pkg_size:x}"
    )]
    DataRegionOutOfBounds {
        /// `data_offset` from the header.
        data_offset: u64,
        /// `data_size` from the header.
        data_size: u64,
        /// `pkg_size` from the header.
        pkg_size: u64,
    },
    /// The encrypted entry table does not fit in the data region.
    #[error("PKG entry table needs 0x{needed:x} bytes, data region is 0x{region:x}")]
    EntryTableOutOfBounds {
        /// Bytes the entry table would occupy.
        needed: usize,
        /// Bytes available in the decrypted data region.
        region: usize,
    },
    /// An item's name field escapes the data region.
    #[error(
        "PKG entry {index}: name [0x{offset:x}, +0x{size:x}) escapes data region 0x{region:x}"
    )]
    NameOutOfBounds {
        /// Zero-based entry index.
        index: usize,
        /// `name_offset` from the record.
        offset: u64,
        /// `name_size` from the record.
        size: u32,
        /// Bytes available in the decrypted data region.
        region: usize,
    },
    /// An item's name field is longer than `MAX_NAME_LEN`.
    #[error("PKG entry {index}: name size 0x{size:x} exceeds 0x{max:x}", max = MAX_NAME_LEN)]
    NameTooLong {
        /// Zero-based entry index.
        index: usize,
        /// `name_size` from the record.
        size: u32,
    },
    /// An item's data field escapes the data region.
    #[error("PKG entry {index} ({name:?}): data [0x{offset:x}, +0x{size:x}) escapes data region 0x{region:x}")]
    FileDataOutOfBounds {
        /// Zero-based entry index.
        index: usize,
        /// Decoded item name.
        name: String,
        /// `file_offset` from the record.
        offset: u64,
        /// `file_size` from the record.
        size: u64,
        /// Bytes available in the decrypted data region.
        region: usize,
    },
    /// An item name contains a path component that would escape the
    /// install root (`..`, an absolute root, or a drive prefix).
    #[error("PKG entry {index}: unsafe item path {name:?}")]
    UnsafePath {
        /// Zero-based entry index.
        index: usize,
        /// The offending decoded name.
        name: String,
    },
}

fn read_be_u16(data: &[u8], off: usize) -> u16 {
    u16::from_be_bytes(
        data[off..off + 2]
            .try_into()
            .expect("invariant: caller bounds-checked this 2-byte read"),
    )
}

fn read_be_u32(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(
        data[off..off + 4]
            .try_into()
            .expect("invariant: caller bounds-checked this 4-byte read"),
    )
}

fn read_be_u64(data: &[u8], off: usize) -> u64 {
    u64::from_be_bytes(
        data[off..off + 8]
            .try_into()
            .expect("invariant: caller bounds-checked this 8-byte read"),
    )
}

/// Parse and validate a retail PKG header. Rejects debug packages,
/// non-PS3 platforms, multi-part packages, and out-of-bounds data
/// regions before any decryption is attempted.
pub fn parse_header(data: &[u8]) -> Result<PkgHeader, PkgError> {
    if data.len() < PKG_HEADER_MIN {
        return Err(PkgError::TooSmall { len: data.len() });
    }
    if data[0..4] != [0x7F, b'P', b'K', b'G'] {
        return Err(PkgError::BadMagic([data[0], data[1], data[2], data[3]]));
    }

    let pkg_type = read_be_u16(data, 0x04);
    match pkg_type {
        PKG_TYPE_RELEASE => {}
        PKG_TYPE_DEBUG => return Err(PkgError::DebugPackage),
        other => return Err(PkgError::UnknownReleaseType(other)),
    }

    let pkg_platform = read_be_u16(data, 0x06);
    if pkg_platform != PKG_PLATFORM_PS3 {
        return Err(PkgError::NonPs3Platform(pkg_platform));
    }

    let file_count = read_be_u32(data, 0x14);
    // Guard the entry-table size computation against overflow.
    if (file_count as u64).checked_mul(ENTRY_LEN as u64).is_none() {
        return Err(PkgError::FileCountTooLarge(file_count));
    }

    let pkg_size = read_be_u64(data, 0x18);
    let data_offset = read_be_u64(data, 0x20);
    let data_size = read_be_u64(data, 0x28);

    if pkg_size > data.len() as u64 {
        return Err(PkgError::SizeMismatch {
            pkg_size,
            len: data.len(),
        });
    }
    if data_offset
        .checked_add(data_size)
        .is_none_or(|end| end > pkg_size)
    {
        return Err(PkgError::DataRegionOutOfBounds {
            data_offset,
            data_size,
            pkg_size,
        });
    }

    let content_id = trim_fixed_str(&data[0x30..0x60]);
    let mut klicensee = [0u8; 16];
    klicensee.copy_from_slice(&data[0x70..0x80]);

    Ok(PkgHeader {
        pkg_type,
        pkg_platform,
        file_count,
        pkg_size,
        data_offset,
        data_size,
        content_id,
        klicensee,
    })
}

/// Trim a fixed-width NUL/space-padded ASCII field to a `String`.
fn trim_fixed_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// Decrypt `region` in place with the package CTR stream: block `b`
/// is XORed with `AES-ECB(PKG_AES_KEY, klicensee + b)`, the counter
/// incrementing per 16-byte block from the data-region start.
fn ctr_decrypt(klicensee: &[u8; 16], region: &mut [u8]) {
    let cipher =
        aes::Aes128::new_from_slice(&PKG_AES_KEY).expect("PKG_AES_KEY is exactly 16 bytes");
    let mut counter = u128::from_be_bytes(*klicensee);
    for block in region.chunks_mut(16) {
        let mut ks = counter.to_be_bytes();
        cipher.encrypt_block((&mut ks).into());
        for (c, k) in block.iter_mut().zip(ks.iter()) {
            *c ^= *k;
        }
        counter = counter.wrapping_add(1);
    }
}

/// Reject any name whose components would escape the install root.
fn validate_item_name(index: usize, name: &str) -> Result<(), PkgError> {
    let unsafe_path = name.starts_with('/')
        || name.starts_with('\\')
        || name.split(['/', '\\']).any(|c| c == "..")
        || name.contains(':');
    if unsafe_path {
        return Err(PkgError::UnsafePath {
            index,
            name: name.to_string(),
        });
    }
    Ok(())
}

/// Parse, decrypt, and extract every in-scope item from a retail PKG.
///
/// The whole data region is CTR-decrypted once and each item's name
/// and data are sliced out of it; this is byte-equivalent to RPCS3's
/// per-region decrypt because PKG item offsets are 16-byte aligned.
/// EDAT and SDAT items are skipped (out of scope). Folder items become
/// [`PkgEntryKind::Directory`] entries.
pub fn extract(data: &[u8]) -> Result<PkgArchive, PkgError> {
    let header = parse_header(data)?;

    // Decrypt the data region in one CTR pass. `fsz` bounds names and
    // data against the actual file extent from `data_offset`, matching
    // RPCS3's `m_file.size() - data_offset`.
    let region_start = header.data_offset as usize;
    let mut region = data[region_start..].to_vec();
    ctr_decrypt(&header.klicensee, &mut region);
    let region_len = region.len();

    let entry_table_len = (header.file_count as usize) * ENTRY_LEN;
    if entry_table_len > region_len {
        return Err(PkgError::EntryTableOutOfBounds {
            needed: entry_table_len,
            region: region_len,
        });
    }

    let mut files = Vec::with_capacity(header.file_count as usize);
    for i in 0..header.file_count as usize {
        let rec = i * ENTRY_LEN;
        let name_offset = read_be_u32(&region, rec) as u64;
        let name_size = read_be_u32(&region, rec + 0x04);
        let file_offset = read_be_u64(&region, rec + 0x08);
        let file_size = read_be_u64(&region, rec + 0x10);
        let entry_type = read_be_u32(&region, rec + 0x18);

        if name_size > MAX_NAME_LEN {
            return Err(PkgError::NameTooLong {
                index: i,
                size: name_size,
            });
        }
        let name_end = name_offset
            .checked_add(name_size as u64)
            .filter(|&end| end <= region_len as u64)
            .ok_or(PkgError::NameOutOfBounds {
                index: i,
                offset: name_offset,
                size: name_size,
                region: region_len,
            })?;
        let name_bytes = &region[name_offset as usize..name_end as usize];
        let name = String::from_utf8_lossy(name_bytes)
            .trim_end_matches('\0')
            .to_string();
        if name.is_empty() {
            continue;
        }
        validate_item_name(i, &name)?;

        match (entry_type & 0xFF) as u8 {
            ENTRY_KIND_FOLDER | ENTRY_KIND_FOLDER_ALT => {
                files.push(PkgFile {
                    name,
                    kind: PkgEntryKind::Directory,
                    data: Vec::new(),
                });
            }
            ENTRY_KIND_EDAT | ENTRY_KIND_SDAT => {
                // DLC / encrypted save containers: out of scope.
            }
            _ => {
                let file_end = file_offset
                    .checked_add(file_size)
                    .filter(|&end| end <= region_len as u64)
                    .ok_or_else(|| PkgError::FileDataOutOfBounds {
                        index: i,
                        name: name.clone(),
                        offset: file_offset,
                        size: file_size,
                        region: region_len,
                    })?;
                let bytes = region[file_offset as usize..file_end as usize].to_vec();
                files.push(PkgFile {
                    name,
                    kind: PkgEntryKind::File,
                    data: bytes,
                });
            }
        }
    }

    Ok(PkgArchive { header, files })
}

#[cfg(test)]
#[path = "tests/pkg_tests.rs"]
mod tests;
