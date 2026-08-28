//! The vocabulary of the vault: keysets, slots, classes, provenance,
//! and what a loader sets aside.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// AES-256-CBC key + IV pair for one SELF key slot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SelfKey {
    /// 32-byte AES-256 key.
    pub erk: [u8; 0x20],
    /// 16-byte initialization vector.
    pub riv: [u8; 0x10],
}

impl fmt::Debug for SelfKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SelfKey { .. }")
    }
}

/// Where a vault value came from, for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The file the value was read from.
    pub path: PathBuf,
    /// 1-based line within a text file; `None` for TOML and raw files.
    pub line: Option<NonZeroUsize>,
}

impl Provenance {
    /// A whole file.
    #[must_use]
    pub fn file(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            line: None,
        }
    }

    /// Line `line` (1-based) of `path`; a 0 is read as the first line.
    #[must_use]
    pub fn at(path: &Path, line: usize) -> Self {
        Self {
            path: path.to_path_buf(),
            line: Some(NonZeroUsize::MIN.saturating_add(line.saturating_sub(1))),
        }
    }

    /// The 1-based line, when there is one.
    #[must_use]
    pub fn line_number(&self) -> Option<usize> {
        self.line.map(NonZeroUsize::get)
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}", self.path.display()),
            None => write!(f, "{}", self.path.display()),
        }
    }
}

/// The scalar key slots the decrypt paths read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// HMAC-SHA1 key over PUP payloads (64 bytes).
    PupHmac,
    /// AES-128 key of the retail PKG CTR keystream (16 bytes).
    PkgAes,
    /// AES-128 key that turns a klicensee into the NPDRM layer key.
    NpKlicKey,
    /// Klicensee of free-license NPDRM titles with no RAP.
    NpKlicFree,
    /// AES-128 key of the RAP derivation's ECB stage.
    RapKey,
    /// Byte permutation of the RAP derivation rounds.
    RapPbox,
    /// First per-round table of the RAP derivation.
    RapE1,
    /// Second per-round table of the RAP derivation.
    RapE2,
}

impl Slot {
    /// Every slot, in report order.
    pub const ALL: [Slot; 8] = [
        Slot::PupHmac,
        Slot::PkgAes,
        Slot::NpKlicKey,
        Slot::NpKlicFree,
        Slot::RapKey,
        Slot::RapPbox,
        Slot::RapE1,
        Slot::RapE2,
    ];

    /// The slot's name in `keys.toml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Slot::PupHmac => "pup_hmac",
            Slot::PkgAes => "pkg_aes",
            Slot::NpKlicKey => "np_klic_key",
            Slot::NpKlicFree => "np_klic_free",
            Slot::RapKey => "rap_key",
            Slot::RapPbox => "rap_pbox",
            Slot::RapE1 => "rap_e1",
            Slot::RapE2 => "rap_e2",
        }
    }

    /// Byte length the slot requires.
    #[must_use]
    pub const fn byte_len(self) -> usize {
        match self {
            Slot::PupHmac => 0x40,
            _ => 0x10,
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Which SELF key table a keyset belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelfClass {
    /// Retail application SELFs (disc titles, firmware modules).
    App,
    /// NPDRM-wrapped SELFs.
    Npdrm,
}

impl fmt::Display for SelfClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SelfClass::App => "app",
            SelfClass::Npdrm => "npdrm",
        })
    }
}

/// Why a file, line, or keyset was read and set aside.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IgnoreReason {
    /// A keyset for a SELF type or key class the decrypt paths never use.
    #[error("keyset {name:?} is not one the decrypt paths use")]
    UnusedKeyset {
        /// Section name, self type, or first table column.
        name: String,
    },
    /// A hex value with no name to file it under.
    #[error("a {bytes}-byte value with no name; name it, or use keys.toml")]
    UnnamedValue {
        /// Decoded length.
        bytes: usize,
    },
    /// A named value whose name matches no slot or key class.
    #[error("name {name:?} matches no key slot")]
    UnrecognizedName {
        /// The name as written.
        name: String,
    },
    /// A file whose extension marks it as not a keyfile.
    #[error("file type .{extension} is not a keyfile")]
    UnsupportedFile {
        /// Lowercased extension.
        extension: String,
    },
    /// A file larger than the walk reads.
    #[error("{bytes} bytes is larger than any keyfile; skipped unread")]
    TooLarge {
        /// File length.
        bytes: u64,
    },
    /// A file that is neither text nor named after a key.
    #[error("binary content under a name that is not a key name")]
    NotText,
    /// A per-title license file; those resolve from `exdata/`, not here.
    #[error("RAP files resolve from the VFS exdata directory, not the vault")]
    RapFile,
    /// A name shaped like one half of a keyset (`<class>-key-<label>`)
    /// whose other half never appeared under the same label.
    #[error("{what:?} looks like a keyset {present}, and no {missing} shares its name")]
    UnpairedHalf {
        /// The half as named.
        what: String,
        /// `erk` or `riv`.
        present: &'static str,
        /// The other one.
        missing: &'static str,
    },
    /// A named value whose length fits no reading of its name.
    #[error("{what:?} is {got} bytes, and its name would need {want}")]
    LengthMismatch {
        /// The value as named.
        what: String,
        /// Decoded length.
        got: usize,
        /// Length the name implies.
        want: usize,
    },
    /// A dot-prefixed file or directory the walk does not open.
    #[error("hidden entry; the walk opens no dot-prefixed file or directory")]
    Hidden,
    /// A directory below the depth the walk descends to.
    #[error("directory {depth} levels down; the walk stops at {max}")]
    TooDeep {
        /// Depth below the vault root.
        depth: usize,
        /// The deepest level the walk opens.
        max: usize,
    },
    /// A file read in full that yielded no key and no other entry.
    #[error("no key found in this file")]
    NothingFound,
}

/// One thing the loader saw and set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// Where.
    pub at: Provenance,
    /// Why.
    pub reason: IgnoreReason,
}
