//! PS3 firmware (PUP) parsing, SELF (SCE) decryption, and TAR extraction.
//!
//! Two consumers share one pipeline: the `cellgov_firmware` binary's
//! `install` subcommand peels the outer SCE/PUP wrapping at install
//! time, and `cellgov_cli`'s boot path calls
//! [`sce::decrypt_self_to_elf`] to peel the inner SELF at load time.
//!
//! Only APP-keyed firmware SELFs are in scope; NPDRM klicensee, RIF,
//! and EDAT are not.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "shared with the cellgov_firmware binary's user-facing output"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod crypto;
pub mod pup;
pub mod sce;
pub mod tar;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pup_header_size_is_0x30() {
        assert_eq!(std::mem::size_of::<pup::PupHeader>(), 0x30);
    }

    #[test]
    fn pup_entry_size_is_0x20() {
        assert_eq!(std::mem::size_of::<pup::PupFileEntry>(), 0x20);
    }

    #[test]
    fn sce_header_size_is_0x20() {
        assert_eq!(std::mem::size_of::<sce::SceContainerHeader>(), 0x20);
    }

    #[test]
    fn metadata_info_size_is_0x40() {
        assert_eq!(std::mem::size_of::<sce::MetadataKeyEnvelope>(), 0x40);
    }

    #[test]
    fn metadata_header_size_is_0x20() {
        assert_eq!(std::mem::size_of::<sce::EncryptedMetadataDirectory>(), 0x20);
    }

    #[test]
    fn metadata_section_header_size_is_0x30() {
        assert_eq!(std::mem::size_of::<sce::EncryptedSectionDescriptor>(), 0x30);
    }
}
