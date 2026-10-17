//! SCE / SELF container format constants for PS3 firmware and signed
//! executables.
//!
//! Behaviour (the decrypter pipeline, the PUP unpacker) lives in
//! `cellgov_install::{sce,pup}`, and key material in the operator's
//! vault (`cellgov_install::keys`); this module is data only.
//!
//! The plaintext constants below are read from nine SELFs, named by
//! path and SHA-256:
//!
//! - Fetched over FTP from the reference console (cech20-cex-493):
//!   - `/dev_flash/vsh/module/vsh.self`
//!     4135c4a5afa2e1ea59676685319d52ef51aa9df1b48b532b098309b922e471d6
//!     (the console's modified copy; the stock 4.93 file is
//!     1a25157ad4cfd9e480f427e9bfa27b60bd784e1f2c08c387061fd9edc1b5dfa1)
//!   - `/dev_flash/sys/external/libfs.sprx`
//!     5db0ad1219496d2bc14554159395812af5dc1c3c70cdea5c62916021689dcca1
//!   - `/dev_flash/bdplayer/bdj.self`
//!     b61339880e8235af54916a596274e5e0e11aa3541840e21a907f3ba3fef5af9c
//!   - a homebrew NPDRM `EBOOT.BIN` under `/dev_hdd0/game`
//!     dbc0b4b5d53edea3f0fc524e6ab2662c355c9dec613399dee7a42932e8258b76
//! - From the 4.93 update package and retail titles:
//!   - `lv2_kernel.self`
//!     e5253ec74b5fc2a05ee16c68e6b6f85dfa192482eef9fe6614eb86e92ea28a9e
//!   - the disc `EBOOT.BIN` of two retail titles
//!     3717dc639891fe738227f078b01033c9599bc9bf0842980b2730be22eaf184bd
//!     and
//!     0ee388eb90a99d2211e793d18c26f9364650d15843df506f39e62ad3e69e0a08
//!   - the NPDRM `EBOOT.BIN` of two retail titles
//!     8a837487c92253eef78cdcd227328bd986e50cd4e09f8cf4150c6cdc40006b36
//!     and
//!     5974598612118161814fadda2634d28f8ca464e54e2f9b343bb89e157ee13bbc
//!
//! All nine open with `SCE\0`, SCE version 2 and header type 1, and
//! place the program identification header at 0x70.
//!
//! The encrypted-metadata constants (the section descriptor, the data
//! key slot, and the encryption and compression kinds) are not
//! console-checkable: only a keyed host decrypts the metadata they
//! describe.

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
///
/// Of the nine SELFs this module names, only the three NPDRM
/// executables carry it: a 0x90-byte record opening `NPD\0` that
/// follows the 0x30-byte type-1 and 0x40-byte type-2 records.
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
///
/// The 4.93 `lv2_kernel.self` reads 3.
pub const SELF_PROGRAM_TYPE_LV2: u32 = 3;

/// `program_type` of an application SELF (a disc executable, a
/// firmware module); the APP keys open it.
///
/// `vsh.self`, `libfs.sprx`, `bdj.self` and both disc executables read 4.
pub const SELF_PROGRAM_TYPE_APP: u32 = 4;

/// `program_type` of an NPDRM-wrapped application SELF.
///
/// All three NPDRM executables read 8, retail and homebrew alike.
pub const SELF_PROGRAM_TYPE_NPDRM: u32 = 8;

/// The `version` word of a program identification header: a major in
/// the top 16 bits, then the minor as two BCD digits (3.55 is
/// `0x0003_0055_0000_0000`).
///
/// The 4.93 kernel, `vsh.self` and `bdj.self` read
/// `0x0004_0093_0000_0000`. The word names the firmware a system SELF
/// was built for: `libfs.sprx` in the same 4.93 flash reads
/// `0x0004_0090_0000_0000`. Every application SELF
/// read (disc, NPDRM and homebrew) carries its own version 1.00,
/// `0x0001_0000_0000_0000`.
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
///
/// Both disc executables, both retail NPDRM executables and the
/// homebrew NPDRM executable read it, with vendor id 0x0100_0002.
pub const RETAIL_APP_PROGRAM_AUTHORITY_ID: u64 = 0x1010_0000_0100_0003;

/// Program authority id of the bdj.self (BD-J / system-process) SELF.
///
/// The console's `/dev_flash/bdplayer/bdj.self` reads it.
pub const BDJ_SELF_PROGRAM_AUTHORITY_ID: u64 = 0x1070_0000_3A00_0001;

/// `supplemental_header.type == 1` marks the plaintext capability
/// header, whose first word is `ctrl_flag1`. The record is 0x30 bytes
/// (0x10 header + 0x20 body) and is readable without decryption.
///
/// Each of the nine SELFs this module names opens its supplemental
/// chain with this 0x30-byte record.
pub const SCE_SUPPLEMENTAL_KIND_PLAINTEXT_CAPABILITY: u32 = 1;

/// A program authority id identifies a CoreOS SELF (vsh and the other
/// system executables) when its top 28 bits equal this value.
/// Compare as `authority_id >> 36`.
///
/// `vsh.self` (0x1070_0005_FF00_0001), `libfs.sprx`
/// (0x1070_0000_4000_0001) and `bdj.self` match. The kernel
/// (0x1050_0000_0300_0001) and every application SELF do not.
pub const COREOS_AUTHORITY_ID_PREFIX: u64 = 0x0107_0000;

/// `ctrl_flags1` mask for root privilege.
///
/// The three capability masks overlap, and their exact bit semantics
/// are unconfirmed even in the reference implementation.
///
/// The files do not settle them. `ctrl_flags1` reads 0x4000_0000 in
/// both copies of `vsh.self` and in the homebrew NPDRM executable, and
/// 0 in `libfs.sprx`, `bdj.self`, the kernel and the four retail
/// titles. System, game and NPDRM SELFs therefore do not separate on
/// the word, and no file sets a bit of [`CTRL_FLAGS1_DEBUG_MASK`].
pub const CTRL_FLAGS1_ROOT_MASK: u32 = 0xC000_0000;

/// `ctrl_flags1` mask for debug-or-root privilege. See
/// [`CTRL_FLAGS1_ROOT_MASK`] on the overlap.
pub const CTRL_FLAGS1_DEBUG_OR_ROOT_MASK: u32 = 0xE000_0000;

/// `ctrl_flags1` mask for debug privilege. See
/// [`CTRL_FLAGS1_ROOT_MASK`] on the overlap.
pub const CTRL_FLAGS1_DEBUG_MASK: u32 = 0xA000_0000;
