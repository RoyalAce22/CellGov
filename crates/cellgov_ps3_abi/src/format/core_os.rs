//! CoreOS package facts: the update package that carries the kernel,
//! and the file table inside it.
//!
//! `CORE_OS_PACKAGE.pkg` is one of the SCE-wrapped packages in a PUP's
//! `update_files` payload, beside the `dev_flash_*` packages. Its
//! decrypted image opens with a fixed header and a table of
//! fixed-width entries, each naming one file and where it sits in the
//! image. The parser lives in `cellgov_install::core_os`; this module
//! is data only.
//!
//! The layout below comes from retail update packages 1.02 through
//! 4.93 (twelve generations); no public document describes it. The set
//! of files the table names changes across generations.

/// Name of the CoreOS package inside the `update_files` payload.
pub const CORE_OS_PACKAGE_NAME: &str = "CORE_OS_PACKAGE.pkg";

/// Bytes of the fixed header at image offset 0.
///
/// Four big-endian words:
///
/// - a format word, observed as 1
/// - the entry count
/// - a word observed as 0
/// - the image length in bytes
pub const CORE_OS_HEADER_SIZE: usize = 0x10;

/// Image offset of the header's format word.
pub const CORE_OS_FORMAT_OFFSET: usize = 0x00;

/// The format word every package read so far opens with.
pub const CORE_OS_FORMAT_WORD: u32 = 1;

/// Image offset of the header's entry-count word.
pub const CORE_OS_ENTRY_COUNT_OFFSET: usize = 0x04;

/// Image offset of the header's image-length word.
pub const CORE_OS_IMAGE_LENGTH_OFFSET: usize = 0x0C;

/// Bytes of one file-table entry: a `u64` offset, a `u64` size, and
/// a NUL-padded name.
pub const CORE_OS_ENTRY_SIZE: usize = 0x30;

/// Entry offset of the file's `u64` image offset.
pub const CORE_OS_ENTRY_OFFSET_FIELD: usize = 0x00;

/// Entry offset of the file's `u64` size.
pub const CORE_OS_ENTRY_SIZE_FIELD: usize = 0x08;

/// Entry offset of the NUL-padded name field.
pub const CORE_OS_ENTRY_NAME_FIELD: usize = 0x10;

/// Bytes of the name field.
pub const CORE_OS_ENTRY_NAME_SIZE: usize = 0x20;

const _: () = assert!(CORE_OS_ENTRY_NAME_FIELD + CORE_OS_ENTRY_NAME_SIZE == CORE_OS_ENTRY_SIZE);

/// The LV2 kernel, as the file table names it.
///
/// Every generation read so far carries it under this name, as an
/// SCE-wrapped SELF.
pub const LV2_KERNEL_SELF: &str = "lv2_kernel.self";
