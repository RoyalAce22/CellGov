//! Refusals of the vault loader and of the accessors the decrypt paths
//! call.

use std::path::PathBuf;

use super::{Provenance, SelfClass, Slot, ENV_KEYS};

/// Why a hex string did not decode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    /// A character outside `0-9a-fA-F` and the tolerated separators.
    #[error("non-hex character {ch:?}")]
    NonHex {
        /// The offending character.
        ch: char,
    },
    /// An odd number of hex digits.
    #[error("odd number of hex digits ({digits})")]
    OddLength {
        /// Digit count after separators are dropped.
        digits: usize,
    },
}

/// Why the vault could not be located, read, or reconciled.
#[derive(Debug, thiserror::Error)]
pub enum KeyVaultError {
    /// Neither the environment nor the VFS names a vault.
    #[error("no key vault: set {ENV_KEYS} to a keys file or directory, or run `cellgov keys import <keys>` (looked for {})", installed.display())]
    NotConfigured {
        /// The installed-vault path that was probed.
        installed: PathBuf,
    },
    /// `CELLGOV_KEYS` is set to an empty string.
    #[error("{ENV_KEYS} is set but empty; point it at a keys file or directory")]
    EnvEmpty,
    /// The named vault path is not there.
    #[error("key vault {} does not exist", path.display())]
    Missing {
        /// The path that was probed.
        path: PathBuf,
    },
    /// Reading a vault file or directory failed.
    #[error("read key vault {}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A `.toml` vault did not parse.
    #[error("{at}: not a keys.toml: {source}")]
    Toml {
        /// The file.
        at: Provenance,
        /// The parser's refusal.
        #[source]
        source: Box<toml::de::Error>,
    },
    /// A value that should be hex is not.
    #[error("{at}: {what}: {source}")]
    BadHex {
        /// Where.
        at: Provenance,
        /// The slot or field the value was for.
        what: String,
        /// The decoding refusal.
        #[source]
        source: HexError,
    },
    /// A value decoded to the wrong number of bytes.
    #[error("{at}: {what} is {got} bytes, needs {want}")]
    WrongLength {
        /// Where.
        at: Provenance,
        /// The slot or field the value was for.
        what: String,
        /// Decoded length.
        got: usize,
        /// Length the slot requires.
        want: usize,
    },
    /// The same slot or label given twice with different values.
    #[error("{what} is given twice with different values: {first} and {second}")]
    Conflict {
        /// Slot name, `<class> revision 0x..`, or `lv2 versions <range>`.
        what: String,
        /// The earlier definition.
        first: Provenance,
        /// The later, disagreeing one.
        second: Provenance,
    },
    /// A `keys.toml` field that names no slot.
    #[error("{at}: unknown field {name:?} (slots: {})", Slot::ALL.map(Slot::name).join(", "))]
    UnknownName {
        /// The file.
        at: Provenance,
        /// The field as written.
        name: String,
    },
    /// A revision that is not a hex number in `0..=0x7fff`.
    #[error("{at}: revision {value:?} is not a hex revision (00..7fff)")]
    BadRevision {
        /// Where.
        at: Provenance,
        /// The value as written.
        value: String,
    },
    /// An LV2 keyset's version label that names no firmware version or
    /// range.
    #[error(
        "{at}: version {value:?} is not a firmware version (3.55, 3.60-3.61, or 16 hex digits)"
    )]
    BadVersion {
        /// Where.
        at: Provenance,
        /// The value as written.
        value: String,
    },
    /// A `keys.toml` keyset whose label field belongs to another class.
    #[error("{at}: [[{class}]] takes {expected}, not {found}")]
    WrongLabelKind {
        /// The file.
        at: Provenance,
        /// The keyset's class.
        class: SelfClass,
        /// The label field this class takes.
        expected: &'static str,
        /// The field the entry carried.
        found: &'static str,
    },
    /// A decrypt path asked for a scalar the vault does not hold.
    #[error("key vault has no {slot} (needed here); check `cellgov keys show`")]
    MissingSlot {
        /// The slot asked for.
        slot: Slot,
    },
    /// A decrypt path asked for an SCE package key and the vault holds none.
    #[error("key vault has no SCE package (scepkg) key; check `cellgov keys show`")]
    MissingScepkg,
}
