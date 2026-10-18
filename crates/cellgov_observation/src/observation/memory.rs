//! Named memory region snapshots taken at end of run.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Region name for the title's `code` PT_LOAD. Single source of
/// truth for callers that look up the ELF-bearing region.
pub const CODE_REGION_NAME: &str = "code";

/// Bytes on one line of a region's `data_hex`.
const HEX_LINE_BYTES: usize = 32;

/// Bytes in one space-separated group of a `data_hex` line: the 4-byte
/// big-endian word the probes and their tests read.
const HEX_WORD_BYTES: usize = 4;

/// The lowercase hex digits, indexed by value.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// A named memory region snapshot taken at end of run.
///
/// All observed regions must be test-owned and write-complete: the test
/// allocates the region, fully initializes it to a known value, and
/// writes its result before terminating. Comparison must not depend on
/// uninitialized or partially-written memory.
///
/// [Martignoni2009 p:131 s:3.2.1] The final-state comparison does not
/// cover the whole address space; only the memory the run wrote is
/// fetched and compared.
///
/// In JSON the bytes are `data_hex`: one string per 32 bytes, in
/// lowercase hex, in space-separated 4-byte groups. The last line and
/// its last group are shorter when the length requires. A reader also
/// accepts `data`, one number per byte, which files from before
/// `data_hex` hold, and refuses a region that carries both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedMemoryRegion {
    /// Region name from the manifest.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Raw bytes captured at end of run.
    pub data: Vec<u8>,
}

/// Why a region's bytes could not be read from JSON.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum RegionDataError {
    /// The region carries neither byte field.
    #[error("region {region:?} has neither `data_hex` nor `data`")]
    Missing {
        /// Region name.
        region: String,
    },
    /// The region carries both byte fields, which could disagree.
    #[error("region {region:?} has both `data_hex` and `data`; keep one")]
    BothForms {
        /// Region name.
        region: String,
    },
    /// A group is empty, of odd length, or holds a character other than
    /// `0-9` and `a-f`.
    #[error("region {region:?} data_hex[{line}]: {group:?} is not lowercase hex bytes")]
    BadGroup {
        /// Region name.
        region: String,
        /// Index of the line in `data_hex`.
        line: usize,
        /// The group as written.
        group: String,
    },
    /// The bytes decode, but a line groups them differently from the
    /// writer, so a rewrite of the file would change it.
    #[error("region {region:?} data_hex[{line}] is {found:?}; the written form is {expected:?}")]
    NotWrittenForm {
        /// Region name.
        region: String,
        /// Index of the line in `data_hex`.
        line: usize,
        /// The line as found, or empty past the end of the file's lines.
        found: String,
        /// The line the writer produces for the same bytes, or empty
        /// past the end of its lines.
        expected: String,
    },
}

/// `data` as `data_hex` lines.
fn hex_lines(data: &[u8]) -> Vec<String> {
    data.chunks(HEX_LINE_BYTES)
        .map(|line| {
            let mut text = String::with_capacity(line.len() * 2 + line.len() / HEX_WORD_BYTES);
            for (i, group) in line.chunks(HEX_WORD_BYTES).enumerate() {
                if i > 0 {
                    text.push(' ');
                }
                for byte in group {
                    // A nibble is below 16, so it indexes every digit.
                    text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
                    text.push(char::from(HEX_DIGITS[usize::from(byte & 0xF)]));
                }
            }
            text
        })
        .collect()
}

/// The value of one lowercase hex digit.
fn hex_digit(c: u8) -> Option<u8> {
    HEX_DIGITS
        .iter()
        .position(|&d| d == c)
        .and_then(|value| u8::try_from(value).ok())
}

/// The bytes `lines` hold. Refuses lines that differ from what
/// [`hex_lines`] writes for those bytes.
fn parse_hex_lines(region: &str, lines: &[String]) -> Result<Vec<u8>, RegionDataError> {
    let mut data = Vec::with_capacity(lines.len() * HEX_LINE_BYTES);
    for (index, line) in lines.iter().enumerate() {
        for group in line.split(' ') {
            let bad = || RegionDataError::BadGroup {
                region: region.to_string(),
                line: index,
                group: group.to_string(),
            };
            let digits = group.as_bytes();
            if digits.is_empty() || digits.len() % 2 != 0 {
                return Err(bad());
            }
            for pair in digits.chunks_exact(2) {
                let &[high, low] = pair else {
                    return Err(bad());
                };
                let (Some(high), Some(low)) = (hex_digit(high), hex_digit(low)) else {
                    return Err(bad());
                };
                data.push(high << 4 | low);
            }
        }
    }
    let written = hex_lines(&data);
    for line in 0..written.len().max(lines.len()) {
        let found = lines.get(line).map_or("", String::as_str);
        let expected = written.get(line).map_or("", String::as_str);
        if found != expected {
            return Err(RegionDataError::NotWrittenForm {
                region: region.to_string(),
                line,
                found: found.to_string(),
                expected: expected.to_string(),
            });
        }
    }
    Ok(data)
}

impl Serialize for NamedMemoryRegion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Written<'a> {
            name: &'a str,
            addr: u64,
            data_hex: Vec<String>,
        }
        Written {
            name: &self.name,
            addr: self.addr,
            data_hex: hex_lines(&self.data),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NamedMemoryRegion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Read {
            name: String,
            addr: u64,
            #[serde(default)]
            data_hex: Option<Vec<String>>,
            #[serde(default)]
            data: Option<Vec<u8>>,
        }
        let Read {
            name,
            addr,
            data_hex,
            data,
        } = Read::deserialize(deserializer)?;
        let data = match (data_hex, data) {
            (Some(lines), None) => parse_hex_lines(&name, &lines),
            (None, Some(bytes)) => Ok(bytes),
            (Some(_), Some(_)) => Err(RegionDataError::BothForms {
                region: name.clone(),
            }),
            (None, None) => Err(RegionDataError::Missing {
                region: name.clone(),
            }),
        }
        .map_err(serde::de::Error::custom)?;
        Ok(Self { name, addr, data })
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
