//! SCE / SELF container format constants for PS3 firmware and signed
//! executables.
//!
//! Behaviour (the decrypter pipeline, the PUP unpacker) lives in
//! `cellgov_install::{sce,pup}`, and key material in the operator's
//! vault (`cellgov_install::keys`); this module is data only.

/// SCE container magic bytes (`"SCE\0"`) at offset 0 of every
/// signed PS3 file.
pub const SCE_MAGIC: [u8; 4] = *b"SCE\0";

/// [`SCE_MAGIC`] as the big-endian word a header parser reads at offset 0.
pub const SCE_MAGIC_U32: u32 = u32::from_be_bytes(SCE_MAGIC);

/// Bytes of one section descriptor in the decrypted metadata directory.
pub const SCE_SECTION_DESCRIPTOR_SIZE: usize = 0x30;

/// Bytes of one slot in the metadata directory's data-key table; a
/// section's key and IV each occupy one.
pub const SCE_DATA_KEY_SIZE: usize = 0x10;

/// `section_kind` value for SCE sections that describe the original
/// program-header table; consumed by the SELF decrypter to rebuild
/// the plaintext ELF's PHDR.
pub const SCE_SECTION_KIND_PHDR: u32 = 2;

/// `encryption_kind = 1`: section payload is stored plaintext (no
/// AES-128-CTR wrapper).
pub const SCE_ENC_KIND_PLAIN: u32 = 1;

/// `encryption_kind = 3`: section payload is wrapped in AES-128-CTR
/// using the section's nested key + IV.
pub const SCE_ENC_KIND_AES128_CTR: u32 = 3;

/// `compression_kind = 1`: section payload is stored uncompressed.
pub const SCE_COMP_KIND_NONE: u32 = 1;

/// `compression_kind = 2`: section payload is zlib-compressed and
/// the consumer must inflate before parsing.
pub const SCE_COMP_KIND_ZLIB: u32 = 2;

/// `supplemental_header.type == 3` marks the NPDRM (NPD) header in
/// an NPDRM-wrapped SELF; presence selects the NPDRM decrypt prefix
/// over the APP-keyed one.
pub const SCE_SUPPLEMENTAL_KIND_NPDRM: u32 = 3;

/// Bytes of a SELF's plaintext program identification header: the
/// authority id, the vendor id, the program type and the version.
pub const SELF_PROGRAM_ID_SIZE: usize = 0x20;

/// Offset of the `program_type` word inside the program identification
/// header.
pub const SELF_PROGRAM_ID_TYPE_OFFSET: usize = 0x0C;

/// Offset of the `version` word inside the program identification
/// header; see [`self_version`] for its layout.
pub const SELF_PROGRAM_ID_VERSION_OFFSET: usize = 0x10;

/// `program_type` of the LV2 kernel SELF; the LV2 loader's keys open it.
pub const SELF_PROGRAM_TYPE_LV2: u32 = 3;

/// `program_type` of an application SELF (a disc executable, a
/// firmware module); the APP keys open it.
pub const SELF_PROGRAM_TYPE_APP: u32 = 4;

/// `program_type` of an NPDRM-wrapped application SELF.
pub const SELF_PROGRAM_TYPE_NPDRM: u32 = 8;

/// The `version` word of a program identification header: the firmware
/// major in the top 16 bits, then the minor as two BCD digits (3.55 is
/// `0x0003_0055_0000_0000`).
#[must_use]
pub const fn self_version(major: u16, minor_bcd: u16) -> u64 {
    ((major as u64) << 48) | ((minor_bcd as u64) << 32)
}

/// The firmware major of a [`self_version`] word.
#[must_use]
pub const fn self_version_major(version: u64) -> u16 {
    (version >> 48) as u16
}

/// The BCD firmware minor of a [`self_version`] word.
#[must_use]
pub const fn self_version_minor(version: u64) -> u16 {
    (version >> 32) as u16
}

/// Program authority id carried by retail application SELFs (disc and
/// NPDRM alike); the boot-identity fallback for a raw ELF with no SELF
/// identification header.
pub const RETAIL_APP_PROGRAM_AUTHORITY_ID: u64 = 0x1010_0000_0100_0003;

/// Program authority id of the bdj.self (BD-J / system-process) SELF.
pub const BDJ_SELF_PROGRAM_AUTHORITY_ID: u64 = 0x1070_0000_3A00_0001;

/// `supplemental_header.type == 1` marks the plaintext capability
/// header, whose first word is `ctrl_flag1`. The record is 0x30 bytes
/// (0x10 header + 0x20 body) and is readable without decryption.
pub const SCE_SUPPLEMENTAL_KIND_PLAINTEXT_CAPABILITY: u32 = 1;

/// A program authority id identifies a CoreOS SELF (vsh and the other
/// system executables) when its top 28 bits equal this value.
/// Compare as `authority_id >> 36`.
pub const COREOS_AUTHORITY_ID_PREFIX: u64 = 0x0107_0000;

/// `ctrl_flags1` mask for root privilege.
///
/// The three capability masks overlap, and their exact bit semantics
/// are unconfirmed even in the reference implementation.
pub const CTRL_FLAGS1_ROOT_MASK: u32 = 0xC000_0000;

/// `ctrl_flags1` mask for debug-or-root privilege. See
/// [`CTRL_FLAGS1_ROOT_MASK`] on the overlap.
pub const CTRL_FLAGS1_DEBUG_OR_ROOT_MASK: u32 = 0xE000_0000;

/// `ctrl_flags1` mask for debug privilege. See
/// [`CTRL_FLAGS1_ROOT_MASK`] on the overlap.
pub const CTRL_FLAGS1_DEBUG_MASK: u32 = 0xA000_0000;
