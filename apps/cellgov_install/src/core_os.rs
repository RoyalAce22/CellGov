//! The file table of a decrypted CoreOS image.
//!
//! The image is what `CORE_OS_PACKAGE.pkg` decrypts to. The table
//! names every file the package holds and where each sits. This module
//! reads the table and bounds each entry against the image; nothing
//! here needs a key.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

use cellgov_ps3_abi::format::core_os::{
    CORE_OS_ENTRY_COUNT_OFFSET, CORE_OS_ENTRY_NAME_FIELD, CORE_OS_ENTRY_NAME_SIZE,
    CORE_OS_ENTRY_OFFSET_FIELD, CORE_OS_ENTRY_SIZE, CORE_OS_ENTRY_SIZE_FIELD,
    CORE_OS_FORMAT_OFFSET, CORE_OS_FORMAT_WORD, CORE_OS_HEADER_SIZE, CORE_OS_IMAGE_LENGTH_OFFSET,
};

use crate::field::{read_be_u32, read_be_u64, usize_from_header, usize_from_u32};

/// One file the table names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOsEntry {
    /// The name, as the table spells it.
    pub name: String,
    /// Image offset of the file's first byte.
    pub offset: u64,
    /// Byte length of the file.
    pub size: u64,
}

impl CoreOsEntry {
    /// The bytes this entry names inside `image`.
    ///
    /// [`parse_table`] bounded every entry, so this is `None` only for
    /// an image other than the one the table came from.
    #[must_use]
    pub fn payload<'a>(&self, image: &'a [u8]) -> Option<&'a [u8]> {
        let start = usize_from_header(self.offset)?;
        let len = usize_from_header(self.size)?;
        image.get(start..)?.get(..len)
    }
}

/// The file table of one image, in table order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreOsTable {
    /// Every entry the table holds.
    pub entries: Vec<CoreOsEntry>,
}

impl CoreOsTable {
    /// The entry named `name`, when the table holds one.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&CoreOsEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Why an image's file table could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CoreOsTableError {
    /// The image is shorter than the fixed header.
    #[error("CoreOS image is {len} bytes, shorter than its {CORE_OS_HEADER_SIZE}-byte header")]
    TooSmall {
        /// Bytes the image holds.
        len: usize,
    },
    /// The header opens with a format word no package read so far
    /// carries, so the buffer is not a CoreOS image this reader knows.
    #[error(
        "CoreOS header format word is 0x{format:x}, not the 0x{CORE_OS_FORMAT_WORD:x} every \
         read package carries"
    )]
    UnknownFormat {
        /// The format word the header carries.
        format: u32,
    },
    /// The header declares more image than the buffer holds, so the
    /// bytes on hand are not the whole image.
    #[error(
        "CoreOS header declares a 0x{declared:x}-byte image, past the 0x{len:x} bytes on hand"
    )]
    DeclaredLengthPastImage {
        /// The length the header declares.
        declared: u64,
        /// Bytes the image holds.
        len: usize,
    },
    /// The entry count names a table that runs past the image.
    #[error("CoreOS table of {count} entries runs past the 0x{len:x}-byte image")]
    TablePastImage {
        /// The entry count the header declares.
        count: u32,
        /// Bytes the image holds.
        len: usize,
    },
    /// An entry's name field is not NUL-padded UTF-8.
    #[error("CoreOS entry {index} has a name that is not UTF-8")]
    NameNotUtf8 {
        /// Zero-based position of the entry in the table.
        index: usize,
    },
    /// An entry's extent leaves the image.
    #[error(
        "CoreOS entry {index} ({name:?}) spans 0x{offset:x}..+0x{size:x}, past the \
         0x{len:x}-byte image"
    )]
    EntryPastImage {
        /// Zero-based position of the entry in the table.
        index: usize,
        /// The name the entry carries.
        name: String,
        /// The offset the entry declares.
        offset: u64,
        /// The size the entry declares.
        size: u64,
        /// Bytes the image holds.
        len: usize,
    },
}

/// Read the file table at the head of a decrypted CoreOS image.
///
/// Every returned entry lies inside `image`, so a caller may take its
/// [`CoreOsEntry::payload`] without a second bound.
///
/// # Errors
///
/// - [`CoreOsTableError::TooSmall`] for a buffer shorter than the header.
/// - [`CoreOsTableError::UnknownFormat`] for a header whose format word
///   is not the one every read package carries.
/// - [`CoreOsTableError::DeclaredLengthPastImage`] for a truncated image.
/// - [`CoreOsTableError::TablePastImage`] for a count the image cannot
///   hold.
/// - [`CoreOsTableError::NameNotUtf8`] and
///   [`CoreOsTableError::EntryPastImage`] for a malformed entry.
pub fn parse_table(image: &[u8]) -> Result<CoreOsTable, CoreOsTableError> {
    let len = image.len();
    if len < CORE_OS_HEADER_SIZE {
        return Err(CoreOsTableError::TooSmall { len });
    }
    let format = read_be_u32(image, CORE_OS_FORMAT_OFFSET);
    if format != CORE_OS_FORMAT_WORD {
        return Err(CoreOsTableError::UnknownFormat { format });
    }
    let count = read_be_u32(image, CORE_OS_ENTRY_COUNT_OFFSET);
    let declared = u64::from(read_be_u32(image, CORE_OS_IMAGE_LENGTH_OFFSET));
    if usize_from_header(declared).is_none_or(|d| d > len) {
        return Err(CoreOsTableError::DeclaredLengthPastImage { declared, len });
    }
    let table_bytes = usize_from_u32(count)
        .checked_mul(CORE_OS_ENTRY_SIZE)
        .and_then(|t| t.checked_add(CORE_OS_HEADER_SIZE))
        .filter(|&end| end <= len)
        .ok_or(CoreOsTableError::TablePastImage { count, len })?;
    let rows = &image[CORE_OS_HEADER_SIZE..table_bytes];

    let mut entries = Vec::with_capacity(usize_from_u32(count));
    for (index, row) in rows.chunks_exact(CORE_OS_ENTRY_SIZE).enumerate() {
        let offset = read_be_u64(row, CORE_OS_ENTRY_OFFSET_FIELD);
        let size = read_be_u64(row, CORE_OS_ENTRY_SIZE_FIELD);
        let name_raw = &row[CORE_OS_ENTRY_NAME_FIELD..][..CORE_OS_ENTRY_NAME_SIZE];
        let name_end = name_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(CORE_OS_ENTRY_NAME_SIZE);
        let name = std::str::from_utf8(&name_raw[..name_end])
            .map_err(|_| CoreOsTableError::NameNotUtf8 { index })?
            .to_string();
        let entry = CoreOsEntry { name, offset, size };
        if entry.payload(image).is_none() {
            return Err(CoreOsTableError::EntryPastImage {
                index,
                name: entry.name,
                offset,
                size,
                len,
            });
        }
        entries.push(entry);
    }
    Ok(CoreOsTable { entries })
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic tables"
)]
#[path = "tests/core_os_tests.rs"]
mod tests;
