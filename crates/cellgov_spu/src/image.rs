//! SPU ELF images embedded in another file's bytes.
//!
//! A PPU executable carries its SPU programs as whole SPU ELF files
//! inside its own data. No marker or section name is required, so an
//! embedded image is found by its ELF header alone: the ELF magic,
//! 32-bit big-endian, `EM_SPU`, and a program-header table whose
//! PT_LOAD segments fit the file and a local store.
//!
//! [CBE-Handbook p:393 s:14.2.2.1 Table 14-1] an SPE-ELF object is ELFCLASS32, ELFDATA2MSB and EM_SPU.

use cellgov_ps3_abi::format::elf::{
    ELFCLASS32, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_MAGIC, EM_SPU,
};

use crate::loader::{parse_spu_elf, SpuElf};
use crate::state::SPU_LS_SIZE;

/// An SPU ELF found inside a larger file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedSpuElf {
    /// Offset of the image's ELF header in the host file.
    pub offset: usize,
    /// The image's header and segments, with offsets relative to
    /// `offset`.
    pub elf: SpuElf,
}

impl EmbeddedSpuElf {
    /// The image's bytes within `host`, the file it was found in.
    pub fn bytes<'a>(&self, host: &'a [u8]) -> &'a [u8] {
        &host[self.offset..self.offset + self.elf.extent]
    }
}

/// Every SPU ELF embedded in `bytes`, by offset. A file that is itself
/// an SPU ELF is one image at offset 0.
///
/// The scan resumes past each image's extent, so no two images overlap.
pub fn find_embedded_spu_elfs(bytes: &[u8]) -> Vec<EmbeddedSpuElf> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(hit) = next_magic(bytes, at) {
        match spu_elf_at(bytes, hit) {
            Some(elf) => {
                at = hit + elf.extent.max(1);
                found.push(EmbeddedSpuElf { offset: hit, elf });
            }
            None => at = hit + 1,
        }
    }
    found
}

/// The next offset at or after `from` where the ELF magic begins.
fn next_magic(bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(ELF_MAGIC.len())
        .position(|w| w == ELF_MAGIC)
        .map(|p| from + p)
}

/// The SPU ELF whose header begins at `offset`, or `None` when the
/// bytes there are not one.
fn spu_elf_at(bytes: &[u8], offset: usize) -> Option<SpuElf> {
    let data = &bytes[offset..];
    if data.get(ELF_EI_CLASS) != Some(&ELFCLASS32) || data.get(ELF_EI_DATA) != Some(&ELFDATA2MSB) {
        return None;
    }
    let elf = parse_spu_elf(data, SPU_LS_SIZE).ok()?;
    (elf.machine == EM_SPU).then_some(elf)
}

#[cfg(test)]
#[path = "tests/image_tests.rs"]
mod tests;
