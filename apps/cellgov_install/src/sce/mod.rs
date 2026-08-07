//! SCE/SELF package decrypter for PS3 firmware and game binaries.
//!
//! All SCE/SELF headers are big-endian. [`decrypt_self_to_elf`] emits a
//! plaintext ELF with both per-segment and outer SCE signatures
//! stripped; the result must not be re-signed or fed to anything that
//! verifies signatures.

mod decrypt;
mod elf;
mod error;
mod raw;

pub use decrypt::{decrypt_package, decrypt_sce_sections, decrypt_self_to_elf};
pub use elf::mask_non_semantic_elf_bytes;
pub use error::SceError;
pub use raw::{
    parse_program_authority_id, parse_sce_header, EncryptedMetadataDirectory,
    EncryptedSectionDescriptor, MetadataKeyEnvelope, SceContainerHeader,
};

pub(crate) use decrypt::{decrypt_envelope, decrypt_sections_from_envelope};
pub(crate) use elf::assemble_elf_from_sections;
pub(crate) use raw::find_supplemental_body;

#[cfg(test)]
#[path = "tests/sce_tests.rs"]
mod tests;
