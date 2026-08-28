//! Operator-supplied key vault: the SELF, package, and NPDRM key
//! material the decrypt paths consume, loaded once from a file or
//! directory the operator owns.
//!
//! The vault is located by [`KeyVault::load`]: the `CELLGOV_KEYS`
//! environment variable names a file or directory used in place, and
//! without it the keys `cellgov_install keys import` normalized into
//! `<vfs>/.cellgov/keys/keys.toml` are read. Parsing lives in every
//! build; only the `decrypt` feature reads the values.
//!
//! Accepted forms, by file:
//!
//! - `*.toml`: the CellGov schema, [`KeyVault::to_toml`]'s output.
//! - Text keyfiles (`keys`, `*.txt`, `*.ini`, any UTF-8 file): scetool
//!   `[keyset]` blocks (`type=`, `self_type=`, `revision=`, `erk=` /
//!   `key=`, `riv=` / `iv=`), `name: HEX` and `name = HEX` lines, and
//!   pasted key-table rows (`app 3.55 0x0A 3.55++ ERK RIV ...`).
//! - Per-key files named `<class>-<part>-<suffix>` (`app-key-0A`,
//!   `npdrm-iv-0A`, `pkg-key`, `pup-hmac`, `np-klic-free`), holding hex
//!   text or the raw bytes.
//!
//! A SELF keyset whose revision is not stated is kept as an unlabeled
//! candidate; the decrypt paths try every candidate for a revision the
//! vault has no labeled key for, and the envelope's zero padding
//! selects the one that fits.
//!
//! Whatever the loose forms leave unplaced is listed with a reason
//! (`cellgov_install keys show`); `keys.toml` is the exact form.

mod error;
mod hex;
mod loader;
mod names;
mod schema;
mod types;
mod vault;

pub use error::{HexError, KeyVaultError};
pub use hex::decode_hex;
pub use types::{IgnoreReason, Ignored, Provenance, SelfClass, SelfKey, Slot};
pub use vault::{installed_keys_dir, KeyVault, ENV_KEYS, INSTALLED_KEYS_FILE};

#[cfg(test)]
#[path = "tests/keys_tests.rs"]
mod tests;
