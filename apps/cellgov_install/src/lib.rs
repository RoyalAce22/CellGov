//! PS3 firmware (PUP) parsing, SELF (SCE) decryption, and TAR extraction.
//!
//! Two consumers share one pipeline: the `cellgov_install` binary's
//! `install` subcommand peels the outer SCE/PUP wrapping at install
//! time, and `cellgov_cli`'s boot path calls
//! [`sce::decrypt_self_to_elf`] (APP-keyed) or
//! [`npdrm::decrypt_self_to_elf_auto`] (auto-detect APP vs NPDRM) to
//! peel the inner SELF at load time.
//!
//! APP-keyed firmware SELFs and RAP-driven NPDRM SELFs are in scope.
//! RIF-only paths (act.dat / IDPS console-identity derivation) and
//! EDAT decryption are not in scope.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shared with the cellgov_install binary's user-facing output"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod crypto;
pub mod disc_crypt;
pub mod game_install;
pub mod game_uninstall;
pub mod iso;
pub mod manifest;
pub mod npdrm;
pub mod param_sfo;
pub mod pkg;
pub mod pup;
pub mod sce;
pub mod tar;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;
