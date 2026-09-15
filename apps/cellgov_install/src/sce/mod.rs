//! SCE/SELF container parsing, and the package decrypter for PS3
//! firmware and game binaries behind the `decrypt` feature.
//!
//! All SCE/SELF headers are big-endian. `decrypt_self_to_elf` emits a
//! plaintext ELF with both per-segment and outer SCE signatures
//! stripped; the result must not be re-signed or fed to anything that
//! verifies signatures.

#[cfg(feature = "decrypt")]
mod decrypt;
mod elf;
mod error;
mod raw;
mod trace;

#[cfg(feature = "decrypt")]
pub use decrypt::{decrypt_package, decrypt_sce_sections, decrypt_self_to_elf};
pub use elf::mask_non_semantic_elf_bytes;
pub use error::SceError;
pub use raw::{
    parse_control_flags1, parse_program_authority_id, parse_sce_header, EncryptedMetadataDirectory,
    EncryptedSectionDescriptor, MetadataKeyEnvelope, SceContainerHeader,
};
pub use trace::{section_trace_enabled, ENV_FW_DEBUG};

#[cfg(all(test, feature = "decrypt"))]
pub(crate) use decrypt::decrypt_envelope;
#[cfg(feature = "decrypt")]
pub(crate) use decrypt::{decrypt_sections_from_envelope, open_envelope_with};
#[cfg(feature = "decrypt")]
pub(crate) use elf::{assemble_elf_from_sections, inner_elf_segment_file_sizes};
pub(crate) use raw::find_supplemental_body;

#[cfg(test)]
#[path = "tests/sce_tests.rs"]
mod tests;

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/inflate_bound_tests.rs"]
mod inflate_bound_tests;
