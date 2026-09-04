//! Which installable container a file holds, decided from its bytes.

/// The container kinds `install` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// A retail PKG, base or update.
    Pkg,
    /// A decrypted ISO9660 disc image.
    Iso,
}

/// Byte offset of the ISO9660 standard identifier: one byte past the
/// volume-descriptor type in the first descriptor.
const ISO_IDENTIFIER_OFFSET: usize = crate::iso::VDS_START_SECTOR * crate::iso::SECTOR + 1;

/// The four bytes every retail PKG opens with.
const PKG_MAGIC: [u8; 4] = [0x7F, b'P', b'K', b'G'];

/// Which container `head` holds, or `None` when it matches neither.
///
/// `head` must carry at least [`SNIFF_LEN`] bytes of the file's start,
/// or the whole file when the file is shorter. A shorter prefix can
/// only answer `Pkg`.
///
/// ISO9660 leaves the first 16 sectors unconstrained, so one file can
/// satisfy both magics. The PKG magic wins that tie.
pub fn sniff(head: &[u8]) -> Option<Container> {
    if head.len() >= PKG_MAGIC.len() && head[0..PKG_MAGIC.len()] == PKG_MAGIC {
        return Some(Container::Pkg);
    }
    let end = ISO_IDENTIFIER_OFFSET + 5;
    if head.len() >= end && &head[ISO_IDENTIFIER_OFFSET..end] == b"CD001" {
        return Some(Container::Iso);
    }
    None
}

/// Bytes [`sniff`] needs to decide both kinds.
pub const SNIFF_LEN: usize = ISO_IDENTIFIER_OFFSET + 5;

#[cfg(test)]
#[path = "tests/container_tests.rs"]
mod tests;
