//! PS3 firmware (PUP) parsing, SELF (SCE) decryption, and TAR extraction.
//!
//! Two consumers share one pipeline: the store commands peel the outer
//! SCE/PUP wrapping at install time, and the boot path calls
//! [`self_image::to_plaintext_elf`] to peel the inner SELF at load
//! time. Both live in `cellgov_cli`. That wrapper routes to
//! `sce::decrypt_self_to_elf`
//! (APP-keyed) or `npdrm::decrypt_self_to_elf_auto` (auto-detect
//! APP vs NPDRM) according to the caller's declared key policy.
//!
//! Every key-consuming path -- the SCE decrypt pipeline, the NPDRM
//! klicensee derivation, PKG content decryption, and PUP HMAC
//! validation -- is behind the `decrypt` cargo feature, off by
//! default. Without it the crate parses containers and passes
//! plaintext images through, and an SCE-wrapped input is refused
//! with [`sce::SceError::DecryptFeatureDisabled`]. The key material
//! itself is the operator's: every decrypt path takes a
//! [`keys::KeyVault`] loaded from a keyfile the operator supplies, and
//! the crate ships no key value.
//!
//! APP-keyed firmware SELFs and RAP-driven NPDRM SELFs are in scope.
//! RIF-only paths (act.dat / IDPS console-identity derivation) and
//! EDAT decryption are not in scope.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the firmware section trace and the scratch-dir drop refusal write to the operator's terminal"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod container;
mod field;
pub mod firmware_install;
pub mod firmware_uninstall;
pub mod firmware_verify;
pub mod game_install;
pub mod game_uninstall;
pub mod iso;
pub mod keys;
pub mod manifest;
pub mod npdrm;
pub mod param_sfo;
pub mod pkg;
pub mod progress;
pub mod pup;
pub mod sce;
pub mod self_image;
pub mod store;
pub mod system_ver;
pub mod tar;

#[cfg(test)]
pub(crate) mod scratch_dir;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/store_lock_tests.rs"]
mod store_lock_tests;
