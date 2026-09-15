//! Decrypted-sections -> plaintext run-image assembly, plus the
//! section-header mask.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

#[cfg(feature = "decrypt")]
use std::ops::Range;

#[cfg(feature = "decrypt")]
use cellgov_ps3_abi::format::elf::{
    ELF64_SHENT_SIZE, ELF_HEADER_SIZE, ELF_MAGIC_U32, ELF_PHENTSIZE,
};
#[cfg(feature = "decrypt")]
use cellgov_ps3_abi::format::sce::SCE_SECTION_KIND_PHDR;

#[cfg(feature = "decrypt")]
use super::error::SceError;
#[cfg(feature = "decrypt")]
use super::raw::{checked_add_oob, checked_mul_oob, EncryptedSectionDescriptor};
#[cfg(feature = "decrypt")]
use crate::field::{read_be_u16, read_be_u32, read_be_u64, usize_from_header, usize_from_u32};

/// Plaintext geometry of a SELF's inner ELF, read from the SELF
/// extended header and the ELF header it points at.
#[cfg(feature = "decrypt")]
struct InnerElf {
    /// The ELF header's bytes in the SELF.
    ehdr: Range<usize>,
    /// The program-header table's bytes in the SELF.
    phdrs: Range<usize>,
    e_shoff: u64,
    e_shnum: usize,
    /// `(p_offset, p_filesz)` per program-header row, in table order.
    segments: Vec<(usize, usize)>,
}

#[cfg(feature = "decrypt")]
fn parse_inner_elf(data: &[u8]) -> Result<InnerElf, SceError> {
    if data.len() < 0x68 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x68,
        });
    }
    let ehdr_out_of_range = SceError::HeaderOffsetOutOfRange {
        what: "SELF ELF header",
    };
    let ehdr_offset = usize_from_header(read_be_u64(data, 0x30)).ok_or(ehdr_out_of_range)?;
    let ehdr_end = checked_add_oob(ehdr_offset, ELF_HEADER_SIZE, "SELF ELF header")?;
    let Some(ehdr) = data.get(ehdr_offset..ehdr_end) else {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF ELF header",
        });
    };
    let inner_magic = read_be_u32(ehdr, 0);
    if inner_magic != ELF_MAGIC_U32 {
        return Err(SceError::InnerElfBadMagic { got: inner_magic });
    }
    // Field offsets below assume ELFCLASS64 (the only value PS3 SELFs use).
    let ei_class = ehdr[4];
    if ei_class != 2 {
        return Err(SceError::BadElfClass { got: ei_class });
    }
    let e_shoff = read_be_u64(ehdr, 0x28);
    let e_phnum = usize::from(read_be_u16(ehdr, 0x38));
    let e_shnum = usize::from(read_be_u16(ehdr, 0x3C));
    let e_phentsize_raw = read_be_u16(ehdr, 0x36);
    let e_shentsize_raw = read_be_u16(ehdr, 0x3A);
    // Per ELF, entsize is "size of one entry" and only meaningful
    // when there are entries. Firmware SPRXes ship with e_shnum = 0
    // and e_shentsize = 0; only validate when the count is non-zero,
    // matching the architectural constants for ELF64
    // (Elf64_Phdr = 0x38, Elf64_Shdr = 0x40).
    if e_phnum > 0 && e_phentsize_raw != 0x38 {
        return Err(SceError::BadElfEntSize {
            what: "e_phentsize",
            got: e_phentsize_raw,
            expected: 0x38,
        });
    }
    if e_shnum > 0 && e_shentsize_raw != 0x40 {
        return Err(SceError::BadElfEntSize {
            what: "e_shentsize",
            got: e_shentsize_raw,
            expected: 0x40,
        });
    }
    // After the entsize checks above, every row has the architectural
    // size. A table with no rows is empty, whatever entsize it declares.
    let phdrs_out_of_range = SceError::HeaderOffsetOutOfRange {
        what: "SELF program headers",
    };
    let phdr_offset = usize_from_header(read_be_u64(data, 0x38)).ok_or(phdrs_out_of_range)?;
    let phdr_table_bytes = checked_mul_oob(e_phnum, ELF_PHENTSIZE, "SELF program headers")?;
    let phdr_end = checked_add_oob(phdr_offset, phdr_table_bytes, "SELF program headers")?;
    let Some(phdr_table) = data.get(phdr_offset..phdr_end) else {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF program headers",
        });
    };

    let segments = phdr_table
        .chunks_exact(ELF_PHENTSIZE)
        .map(|row| {
            usize_from_header(read_be_u64(row, 0x08))
                .zip(usize_from_header(read_be_u64(row, 0x20)))
                .ok_or(SceError::HeaderOffsetOutOfRange {
                    what: "SELF program header row",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(InnerElf {
        ehdr: ehdr_offset..ehdr_end,
        phdrs: phdr_offset..phdr_end,
        e_shoff,
        e_shnum,
        segments,
    })
}

/// `p_filesz` per program header of `data`'s inner ELF, in table order.
///
/// The decrypt pass bounds a zlib section's inflate output at the size
/// its destination segment declares.
#[cfg(feature = "decrypt")]
pub(crate) fn inner_elf_segment_file_sizes(data: &[u8]) -> Result<Vec<usize>, SceError> {
    Ok(parse_inner_elf(data)?
        .segments
        .into_iter()
        .map(|(_, p_filesz)| p_filesz)
        .collect())
}

/// Reassemble a plaintext ELF from decrypted SCE sections.
///
/// Layout: ehdr as-is, program headers packed immediately after,
/// each PHDR-kind section copied to its declared `p_offset`, and
/// last -- if the SELF's `shdr_offset` and the inner ELF's `e_shoff`
/// are both non-zero -- the original section-header table copied to
/// `e_shoff`.
#[cfg(feature = "decrypt")]
pub(crate) fn assemble_elf_from_sections(
    data: &[u8],
    sections: &[(EncryptedSectionDescriptor, Vec<u8>)],
) -> Result<Vec<u8>, SceError> {
    let InnerElf {
        ehdr,
        phdrs,
        e_shoff,
        e_shnum,
        segments,
    } = parse_inner_elf(data)?;
    let shdr_offset_in_self = read_be_u64(data, 0x40);
    let e_phnum = segments.len();

    let mut elf_size: usize =
        checked_add_oob(ELF_HEADER_SIZE, phdrs.len(), "reconstructed ELF size")?;
    for &(p_offset, p_filesz) in &segments {
        let end = checked_add_oob(p_offset, p_filesz, "SELF program segment extent")?;
        if end > elf_size {
            elf_size = end;
        }
    }
    let shdr_table_bytes = checked_mul_oob(e_shnum, ELF64_SHENT_SIZE, "SELF section headers")?;
    // A null `e_shoff` with a non-zero `e_shnum` leaves the
    // section-header table nowhere to land. The table is no part of
    // the run image (see [`mask_non_semantic_elf_bytes`]), so the
    // shape is dropped. `shdr_table` is `(destination, source)`.
    let shdr_table = if shdr_offset_in_self != 0 && e_shnum > 0 && e_shoff != 0 {
        let shdrs_out_of_range = || SceError::HeaderOffsetOutOfRange {
            what: "SELF section headers",
        };
        let dst_start = usize_from_header(e_shoff).ok_or_else(shdrs_out_of_range)?;
        let src_start = usize_from_header(shdr_offset_in_self).ok_or_else(shdrs_out_of_range)?;
        let dst_end = checked_add_oob(dst_start, shdr_table_bytes, "SELF section headers")?;
        let src_end = checked_add_oob(src_start, shdr_table_bytes, "SELF section headers")?;
        if src_end > data.len() {
            return Err(shdrs_out_of_range());
        }
        if dst_end > elf_size {
            elf_size = dst_end;
        }
        Some((dst_start..dst_end, src_start..src_end))
    } else {
        None
    };

    // `elf_size` is the maximum of file-derived `p_offset + p_filesz`
    // extents, so a corrupt program header can name an image far larger
    // than any host can back. A plain `vec![0u8; elf_size]` would abort
    // the process on allocation failure; the fallible reserve turns
    // that into a named refusal.
    let mut elf: Vec<u8> = Vec::new();
    elf.try_reserve_exact(elf_size)
        .map_err(|_| SceError::ReconstructedElfTooLarge { elf_size })?;
    elf.resize(elf_size, 0);
    elf[..ELF_HEADER_SIZE].copy_from_slice(&data[ehdr]);
    elf[ELF_HEADER_SIZE..][..phdrs.len()].copy_from_slice(&data[phdrs]);
    // Rewrite e_phoff to the packed phdr position; the inner ELF's
    // original value may differ from 0x40.
    elf[0x20..0x28].copy_from_slice(&0x40u64.to_be_bytes());

    for (sec, sec_data) in sections {
        if sec.section_kind != SCE_SECTION_KIND_PHDR {
            continue;
        }
        // No early exit on an empty payload. A zero-length section
        // against a non-zero `p_filesz` is exactly the shape the size
        // check below exists to name; skipping it would leave the
        // segment as silent zeroes in the run image.
        let prog_idx = usize_from_u32(sec.program_segment_index);
        let &(p_offset, p_filesz) = segments
            .get(prog_idx)
            .ok_or(SceError::SectionProgramIndexOutOfRange { prog_idx, e_phnum })?;
        if sec_data.len() != p_filesz {
            return Err(SceError::SectionSizeMismatch {
                prog_idx,
                got: sec_data.len(),
                expected: p_filesz,
            });
        }
        let write_end =
            p_offset
                .checked_add(p_filesz)
                .ok_or(SceError::SectionPastReconstructedElf {
                    prog_idx,
                    offset: p_offset,
                    size: p_filesz,
                    elf_size: elf.len(),
                })?;
        if write_end > elf.len() {
            return Err(SceError::SectionPastReconstructedElf {
                prog_idx,
                offset: p_offset,
                size: p_filesz,
                elf_size: elf.len(),
            });
        }
        elf[p_offset..write_end].copy_from_slice(sec_data);
    }

    // Placed after the segment payloads, so a section-header table
    // whose `e_shoff` falls inside a segment's
    // `[p_offset, p_offset + p_filesz)` overwrites that part of the
    // payload.
    if let Some((dst, src)) = shdr_table {
        elf[dst].copy_from_slice(&data[src]);
    }

    let magic = u32::from_be_bytes([elf[0], elf[1], elf[2], elf[3]]);
    if magic != ELF_MAGIC_U32 {
        return Err(SceError::ReconstructedBadMagic { got: magic });
    }

    Ok(elf)
}

/// Zero `e_shoff`, `e_shnum`, and `e_shstrndx` so a reconstructed
/// ELF can be byte-compared modulo section-header layout.
///
/// The masked fields describe the section / link view. `e_phoff` and
/// the program-header table describe the segment / execution view,
/// which the loader consumes to build the run image, and stay
/// unmasked.
pub fn mask_non_semantic_elf_bytes(elf: &mut [u8]) {
    if elf.len() < 0x40 {
        return;
    }
    elf[0x28..0x30].copy_from_slice(&0u64.to_be_bytes());
    elf[0x3C..0x3E].copy_from_slice(&0u16.to_be_bytes());
    elf[0x3E..0x40].copy_from_slice(&0u16.to_be_bytes());
}
