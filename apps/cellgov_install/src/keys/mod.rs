//! Operator-supplied key vault: the SELF, package, and NPDRM key
//! material the decrypt paths consume, loaded once from a file or
//! directory the operator owns.
//!
//! The vault is located by [`KeyVault::load`]: the `CELLGOV_KEYS`
//! environment variable names a file or directory used in place, and
//! without it the keys `cellgov keys import` normalized into
//! `<vfs>/.cellgov/keys/keys.toml` are read. Parsing lives in every
//! build; only the `decrypt` feature reads the values.
//!
//! Accepted forms, by file:
//!
//! - `*.toml`: the CellGov schema, [`KeyVault::to_toml`]'s output.
//! - Text keyfiles (`keys`, `*.txt`, `*.ini`, any UTF-8 file): scetool
//!   `[keyset]` blocks (`type=`, `self_type=`, `revision=`, `version=`,
//!   `erk=` / `key=`, `riv=` / `iv=`), `name: HEX` and `name = HEX`
//!   lines, and pasted key-table rows (`app 3.55 0x0A 3.55++ ERK RIV
//!   ...`, `lv2 3.60-3.61 ERK RIV`).
//! - Per-key files named `<class>-<part>-<suffix>` (`app-key-0A`,
//!   `npdrm-iv-0A`, `lv2-key-3.60-3.61`, `pkg-key`, `pup-hmac`,
//!   `np-klic-free`), holding hex text or the raw bytes.
//!
//! A key revision labels an APP or NPDRM keyset. The firmware versions
//! an LV2 keyset opens ([`Lv2Versions`]) label it; the kernel SELF's
//! program identification header names them. The loader keeps a SELF
//! keyset with no stated label as an unlabeled candidate. For a SELF
//! the vault has no labeled key for, the decrypt paths try every
//! candidate, and the envelope's zero padding selects the one that
//! fits.
//!
//! Whatever the loose forms leave unplaced is listed with a reason
//! (`cellgov keys show`); `keys.toml` is the exact form.

mod error;
mod hex;
mod loader;
mod lv2_version;
mod names;
mod schema;
mod types;
mod vault;

pub use error::{HexError, KeyVaultError};
pub use hex::decode_hex;
pub use lv2_version::{version_label, Lv2Versions};
pub use types::{CryptoMaterial, IgnoreReason, Ignored, Provenance, SelfClass, SelfKey, Slot};
pub use vault::{installed_keys_dir, KeyVault, ENV_KEYS, INSTALLED_KEYS_FILE};

#[cfg(test)]
#[path = "tests/keys_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/lv2_keys_tests.rs"]
mod lv2_keys_tests;
