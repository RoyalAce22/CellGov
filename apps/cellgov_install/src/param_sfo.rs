//! `PARAM.SFO` parser -- the small key/value metadata table every PS3
//! title carries (title id, category, title string, version). Both
//! game-install inputs (PKG and ISO) read it to learn the install
//! destination and to seed the generated title manifest.
//!
//! Unlike every other PS3 container in this crate, all multi-byte SFO
//! fields are **little-endian**.

use std::collections::BTreeMap;

/// Header magic in on-disk byte order (`\0PSF`).
const SFO_MAGIC: [u8; 4] = [0x00, b'P', b'S', b'F'];
/// The only format version the reference reader accepts.
const SFO_VERSION: u32 = 0x0101;
/// Fixed header size; also the minimum legal key-table offset.
const HEADER_LEN: usize = 0x14;
/// Size of one index record (`def_table_t`).
const INDEX_LEN: usize = 0x10;

/// Data format tag: non-NUL-terminated char array.
const FMT_ARRAY: u16 = 0x0004;
/// Data format tag: NUL-terminated UTF-8 string.
const FMT_STRING: u16 = 0x0204;
/// Data format tag: little-endian `u32`.
const FMT_INTEGER: u16 = 0x0404;

/// One decoded PARAM.SFO value, tagged by its on-disk format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SfoValue {
    /// `format::integer` (0x0404): a little-endian `u32`.
    Integer(u32),
    /// `format::string` (0x0204): UTF-8, trimmed at the first NUL.
    Text(String),
    /// `format::array` (0x0004): raw, non-NUL-terminated bytes.
    Array(Vec<u8>),
}

/// A parsed PARAM.SFO: its key/value entries in sorted key order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamSfo {
    entries: BTreeMap<String, SfoValue>,
}

impl ParamSfo {
    /// Look up a raw entry by key.
    pub fn get(&self, key: &str) -> Option<&SfoValue> {
        self.entries.get(key)
    }

    /// Value of a `Text`-typed key, or `None` if absent or another type.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.entries.get(key)? {
            SfoValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Value of an `Integer`-typed key, or `None` if absent or another type.
    pub fn get_integer(&self, key: &str) -> Option<u32> {
        match self.entries.get(key)? {
            SfoValue::Integer(v) => Some(*v),
            _ => None,
        }
    }

    /// Iterate entries in sorted key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SfoValue)> {
        self.entries.iter()
    }

    /// Number of decoded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has zero decoded entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Why PARAM.SFO parsing failed. Structural corruption is rejected;
/// entries carrying an unknown format tag are skipped, not errored.
#[derive(Debug, thiserror::Error)]
pub enum SfoError {
    /// Input is shorter than the 20-byte header.
    #[error("PARAM.SFO too small for header (got {len} bytes, need 20)")]
    TooSmall {
        /// Observed input length.
        len: usize,
    },
    /// Magic was not `\0PSF`; carries the first 4 observed bytes.
    #[error("bad PARAM.SFO magic: {:02x}{:02x}{:02x}{:02x}", _0[0], _0[1], _0[2], _0[3])]
    BadMagic([u8; 4]),
    /// `version` field was not the supported `0x101`.
    #[error("unsupported PARAM.SFO version 0x{found:x} (expected 0x101)")]
    UnsupportedVersion {
        /// `version` value read from the header.
        found: u32,
    },
    /// `off_key_table` is below the header or above `off_data_table`.
    #[error("key-table offset 0x{off:x} out of range (must be 20..=data-table 0x{data_table:x})")]
    KeyTableOutOfRange {
        /// `off_key_table` value read from the header.
        off: u32,
        /// `off_data_table` value read from the header.
        data_table: u32,
    },
    /// `off_data_table` exceeds the file length.
    #[error("data-table offset 0x{off:x} exceeds file length 0x{len:x}")]
    DataTableOutOfRange {
        /// `off_data_table` value read from the header.
        off: u32,
        /// Actual input length.
        len: usize,
    },
    /// The index table would extend past the file end.
    #[error("index table needs 0x{required:x} bytes, file is 0x{len:x}")]
    IndexTruncated {
        /// Byte length the header plus index table would occupy.
        required: usize,
        /// Actual input length.
        len: usize,
    },
    /// An entry's `key_off` points outside the key-name region.
    #[error(
        "entry {index}: key offset 0x{key_off:x} escapes key table (0x{key_region_len:x} bytes)"
    )]
    KeyOffsetOutOfRange {
        /// Zero-based index of the offending entry.
        index: usize,
        /// The out-of-range `key_off`.
        key_off: u16,
        /// Length of the key-name region.
        key_region_len: usize,
    },
    /// An entry's key name has no NUL terminator before the region end.
    #[error("entry {index}: key name at 0x{key_off:x} is not NUL-terminated")]
    KeyNotTerminated {
        /// Zero-based index of the offending entry.
        index: usize,
        /// The entry's `key_off`.
        key_off: u16,
    },
    /// Two entries decoded to the same key name.
    #[error("duplicate PARAM.SFO key {0:?}")]
    DuplicateKey(String),
    /// An entry's used length exceeds its allocated length.
    #[error("entry {key:?}: param_len 0x{param_len:x} exceeds param_max 0x{param_max:x}")]
    ParamLenExceedsMax {
        /// The offending key name.
        key: String,
        /// `param_len` (used bytes).
        param_len: u32,
        /// `param_max` (allocated bytes).
        param_max: u32,
    },
    /// An entry's data region escapes the file bounds.
    #[error("entry {key:?}: data [0x{start:x}, 0x{end:x}) escapes file length 0x{len:x}")]
    DataOutOfRange {
        /// The offending key name.
        key: String,
        /// Absolute start offset of the data region.
        start: usize,
        /// Absolute end offset of the data region.
        end: usize,
        /// Actual input length.
        len: usize,
    },
}

fn read_le_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(
        data[off..off + 2]
            .try_into()
            .expect("invariant: caller bounds-checked this 2-byte read"),
    )
}

fn read_le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(
        data[off..off + 4]
            .try_into()
            .expect("invariant: caller bounds-checked this 4-byte read"),
    )
}

/// Parse a PARAM.SFO image. Rejects structural corruption (bad magic /
/// version, out-of-range offsets, truncated tables, duplicate keys);
/// silently skips entries whose format tag is none of the three known
/// formats.
pub fn parse(data: &[u8]) -> Result<ParamSfo, SfoError> {
    if data.len() < HEADER_LEN {
        return Err(SfoError::TooSmall { len: data.len() });
    }
    let magic: [u8; 4] = data[0..4]
        .try_into()
        .expect("invariant: data.len() >= 20 guarantees a 4-byte magic");
    if magic != SFO_MAGIC {
        return Err(SfoError::BadMagic(magic));
    }
    let version = read_le_u32(data, 0x04);
    if version != SFO_VERSION {
        return Err(SfoError::UnsupportedVersion { found: version });
    }
    let off_key_table = read_le_u32(data, 0x08);
    let off_data_table = read_le_u32(data, 0x0C);
    let entries_num = read_le_u32(data, 0x10);

    // Header-level bounds.
    if (off_key_table as usize) < HEADER_LEN || off_key_table > off_data_table {
        return Err(SfoError::KeyTableOutOfRange {
            off: off_key_table,
            data_table: off_data_table,
        });
    }
    if off_data_table as usize > data.len() {
        return Err(SfoError::DataTableOutOfRange {
            off: off_data_table,
            len: data.len(),
        });
    }

    // The index table occupies [HEADER_LEN, HEADER_LEN + entries_num*16)
    // and must be fully present to read each record.
    let index_bytes = (entries_num as usize)
        .checked_mul(INDEX_LEN)
        .and_then(|n| HEADER_LEN.checked_add(n))
        .ok_or(SfoError::IndexTruncated {
            required: usize::MAX,
            len: data.len(),
        })?;
    if index_bytes > data.len() {
        return Err(SfoError::IndexTruncated {
            required: index_bytes,
            len: data.len(),
        });
    }

    let key_region = &data[off_key_table as usize..off_data_table as usize];
    let data_table_start = off_data_table as usize;

    let mut entries: BTreeMap<String, SfoValue> = BTreeMap::new();
    for i in 0..entries_num as usize {
        let rec = HEADER_LEN + i * INDEX_LEN;
        let key_off = read_le_u16(data, rec);
        let param_fmt = read_le_u16(data, rec + 0x02);
        let param_len = read_le_u32(data, rec + 0x04);
        let param_max = read_le_u32(data, rec + 0x08);
        let data_off = read_le_u32(data, rec + 0x0C);

        if key_off as usize >= key_region.len() {
            return Err(SfoError::KeyOffsetOutOfRange {
                index: i,
                key_off,
                key_region_len: key_region.len(),
            });
        }
        let key_bytes = &key_region[key_off as usize..];
        let nul = key_bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or(SfoError::KeyNotTerminated { index: i, key_off })?;
        let key = String::from_utf8_lossy(&key_bytes[..nul]).into_owned();

        if param_len > param_max {
            return Err(SfoError::ParamLenExceedsMax {
                key,
                param_len,
                param_max,
            });
        }

        // Unknown format tags are skipped rather than errored.
        let value = match param_fmt {
            FMT_INTEGER if param_max == 4 && param_len == 4 => {
                let start = data_table_start.saturating_add(data_off as usize);
                let end = start.saturating_add(4);
                if end > data.len() {
                    return Err(SfoError::DataOutOfRange {
                        key,
                        start,
                        end,
                        len: data.len(),
                    });
                }
                SfoValue::Integer(read_le_u32(data, start))
            }
            FMT_STRING | FMT_ARRAY => {
                let start = data_table_start.saturating_add(data_off as usize);
                let end = start.saturating_add(param_len as usize);
                if end > data.len() {
                    return Err(SfoError::DataOutOfRange {
                        key,
                        start,
                        end,
                        len: data.len(),
                    });
                }
                let raw = &data[start..end];
                if param_fmt == FMT_STRING {
                    let nul = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                    SfoValue::Text(String::from_utf8_lossy(&raw[..nul]).into_owned())
                } else {
                    SfoValue::Array(raw.to_vec())
                }
            }
            _ => continue,
        };

        if entries.insert(key.clone(), value).is_some() {
            return Err(SfoError::DuplicateKey(key));
        }
    }

    Ok(ParamSfo { entries })
}

#[cfg(test)]
#[path = "tests/param_sfo_tests.rs"]
mod tests;
