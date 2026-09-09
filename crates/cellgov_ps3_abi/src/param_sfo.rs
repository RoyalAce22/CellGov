//! `PARAM.SFO` binary-format facts: the key/value metadata table every
//! title tree carries.
//!
//! No public specification covers the table; the layout below is the
//! one every retail `PARAM.SFO` follows. Unlike the other PS3 container
//! formats, every multi-byte field is little-endian.

/// The table's filename inside a title tree.
pub const PARAM_SFO_FILE: &str = "PARAM.SFO";

/// Key of the lowest system software the title runs on, as a
/// `MM.mmmm` string (`01.5000` names firmware 1.50).
pub const PS3_SYSTEM_VER_KEY: &str = "PS3_SYSTEM_VER";

/// Header magic in on-disk byte order (`\0PSF`).
pub const SFO_MAGIC: [u8; 4] = [0x00, b'P', b'S', b'F'];

/// The one format version retail tables carry.
pub const SFO_FORMAT_VERSION: u32 = 0x0101;

/// Fixed header size: magic, version, key-table offset, data-table
/// offset, entry count. Also the lowest legal key-table offset.
pub const SFO_HEADER_LEN: usize = 0x14;

/// Size of one index record: key offset (u16), format tag (u16), value
/// length (u32), value capacity (u32), data offset (u32).
pub const SFO_INDEX_LEN: usize = 0x10;

/// Format tag: a NUL-terminated UTF-8 string.
pub const SFO_FMT_STRING: u16 = 0x0204;
