//! `PARAM.SFO` binary-format facts: the key/value metadata table every
//! title tree carries.
//!
//! No public specification covers the table; the layout below is the
//! one every retail `PARAM.SFO` follows. Unlike the other PS3 container
//! formats, every multi-byte field is little-endian.
//!
//! Five tables were read, named by SHA-256:
//!
//! - a homebrew title's `PARAM.SFO` under the reference console's
//!   `/dev_hdd0/game`
//!   6219c81f1465766526929929f755d9a11ff5e1ae244ccf0221ac479d31ac261e
//! - two disc titles:
//!   0d12f81e1752536518903ead50a25f88c5adb17980806624866da6aa5c211be8
//!   and
//!   c7804e6afd838a3b9bd3802294243083d599652bdff5cc6d2c543c1e94bd2d2a
//! - two installed NPDRM titles:
//!   79be03c21c5d51ba2184a9606dcd44c81616d9b875985ea9f383828758578849
//!   and
//!   3868b20bcc313050b185ff51c650911d12a2aa3c2773bc7e1a676f2ec4292c08
//!
//! All five open with [`SFO_MAGIC`] and version 0x0101. Their key table
//! starts [`SFO_HEADER_LEN`] plus [`SFO_INDEX_LEN`] bytes per entry from
//! the start. Their only format tags are 0x0204 and 0x0404.
//! `PS3_SYSTEM_VER` is a 0x0204 string in each: `01.5000`, `01.9400`,
//! `02.7600`, `03.4000` and `03.7000`.

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
