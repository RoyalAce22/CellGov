//! Decrypted-sections -> plaintext run-image assembly.

use cellgov_ps3_abi::sce::SCE_SECTION_KIND_PHDR;

use super::error::SceError;
use super::raw::{
    checked_add_oob, checked_mul_oob, read_be_u16, read_be_u32, read_be_u64,
    EncryptedSectionDescriptor,
};

/// Reassemble a plaintext ELF from decrypted SCE sections.
///
/// Layout: ehdr as-is, program headers packed immediately after,
/// each PHDR-kind section copied to its declared `p_offset`, then
/// -- if the SELF's `shdr_offset` is non-zero -- the original
/// section-header table copied to the ELF's declared `e_shoff`.
/// Shared by [`super::decrypt_self_to_elf`] (APP-keyed) and
/// [`crate::npdrm::decrypt_self_to_elf_npdrm`] (NPDRM-keyed); only
/// the envelope path upstream differs.
pub(crate) fn assemble_elf_from_sections(
    data: &[u8],
    sections: &[(EncryptedSectionDescriptor, Vec<u8>)],
) -> Result<Vec<u8>, SceError> {
    if data.len() < 0x68 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x68,
        });
    }
    let ehdr_offset = read_be_u64(data, 0x30) as usize;
    let phdr_offset = read_be_u64(data, 0x38) as usize;
    let shdr_offset_in_self = read_be_u64(data, 0x40) as usize;

    let ehdr_end = checked_add_oob(ehdr_offset, 0x40, "SELF ELF header")?;
    if ehdr_end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF ELF header",
        });
    }
    let inner_magic = read_be_u32(data, ehdr_offset);
    if inner_magic != 0x7F45_4C46 {
        return Err(SceError::InnerElfBadMagic { got: inner_magic });
    }
    // Field offsets below assume ELFCLASS64 (the only value PS3 SELFs use).
    let ei_class = data[ehdr_offset + 4];
    if ei_class != 2 {
        return Err(SceError::BadElfClass { got: ei_class });
    }
    let e_shoff = read_be_u64(data, ehdr_offset + 0x28) as usize;
    let e_phnum = read_be_u16(data, ehdr_offset + 0x38) as usize;
    let e_shnum = read_be_u16(data, ehdr_offset + 0x3C) as usize;
    let e_phentsize_raw = read_be_u16(data, ehdr_offset + 0x36);
    let e_shentsize_raw = read_be_u16(data, ehdr_offset + 0x3A);
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
    let e_phentsize = e_phentsize_raw as usize;
    let e_shentsize = e_shentsize_raw as usize;
    let phdr_table_bytes = checked_mul_oob(e_phnum, e_phentsize, "SELF program headers")?;
    let phdr_end = checked_add_oob(phdr_offset, phdr_table_bytes, "SELF program headers")?;
    if phdr_end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF program headers",
        });
    }

    let mut elf_size: usize = checked_add_oob(0x40, phdr_table_bytes, "reconstructed ELF size")?;
    for i in 0..e_phnum {
        let row_off = checked_mul_oob(i, e_phentsize, "SELF program header row")?;
        let ph_off = checked_add_oob(phdr_offset, row_off, "SELF program header row")?;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        let end = checked_add_oob(p_offset, p_filesz, "SELF program segment extent")?;
        if end > elf_size {
            elf_size = end;
        }
    }
    let shdr_table_bytes = checked_mul_oob(e_shnum, e_shentsize, "SELF section headers")?;
    if shdr_offset_in_self != 0 && e_shnum > 0 {
        let shdr_end = checked_add_oob(e_shoff, shdr_table_bytes, "SELF section headers")?;
        if shdr_end > elf_size {
            elf_size = shdr_end;
        }
        let shdr_end_in_self = checked_add_oob(
            shdr_offset_in_self,
            shdr_table_bytes,
            "SELF section headers",
        )?;
        if shdr_end_in_self > data.len() {
            return Err(SceError::HeaderOffsetOutOfRange {
                what: "SELF section headers",
            });
        }
    }

    let mut elf = vec![0u8; elf_size];
    elf[..0x40].copy_from_slice(&data[ehdr_offset..ehdr_offset + 0x40]);
    let phdr_dst = 0x40usize;
    elf[phdr_dst..phdr_dst + phdr_table_bytes]
        .copy_from_slice(&data[phdr_offset..phdr_offset + phdr_table_bytes]);
    // Rewrite e_phoff to the packed phdr position; the inner ELF's
    // original value may differ from 0x40.
    elf[0x20..0x28].copy_from_slice(&(phdr_dst as u64).to_be_bytes());

    if shdr_offset_in_self != 0 && e_shnum > 0 {
        elf[e_shoff..e_shoff + shdr_table_bytes]
            .copy_from_slice(&data[shdr_offset_in_self..shdr_offset_in_self + shdr_table_bytes]);
    }

    for (sec, sec_data) in sections {
        if sec.section_kind != SCE_SECTION_KIND_PHDR {
            continue;
        }
        if sec_data.is_empty() {
            continue;
        }
        let prog_idx = sec.program_segment_index as usize;
        if prog_idx >= e_phnum {
            return Err(SceError::SectionProgramIndexOutOfRange { prog_idx, e_phnum });
        }
        let row_off = checked_mul_oob(prog_idx, e_phentsize, "SELF program header row")?;
        let ph_off = checked_add_oob(phdr_offset, row_off, "SELF program header row")?;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
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

    let magic = u32::from_be_bytes([elf[0], elf[1], elf[2], elf[3]]);
    if magic != 0x7F454C46 {
        return Err(SceError::ReconstructedBadMagic { got: magic });
    }

    Ok(elf)
}

/// Zero `e_shoff`, `e_shnum`, and `e_shstrndx` so a reconstructed
/// ELF can be byte-compared modulo section-header layout.
///
/// The contract is "identical run image," not "identical ELF
/// file": `e_shoff` / `e_shnum` / `e_shstrndx` describe the
/// section / link view, not part of the run image. `e_phoff` and
/// the program-header table describe the segment / execution
/// view, which the loader consumes to build the run image, and
/// stay unmasked.
///
/// The three-field set is empirically sufficient for the current
/// title corpus (flOw / SSHD / WipEout plus the firmware-PRX
/// parity gate); not proven-minimal against arbitrary PS3 SELFs.
/// Widen when a corpus addition surfaces a fourth non-semantic
/// ELF64 header field.
pub fn mask_non_semantic_elf_bytes(elf: &mut [u8]) {
    if elf.len() < 0x40 {
        return;
    }
    elf[0x28..0x30].copy_from_slice(&0u64.to_be_bytes());
    elf[0x3C..0x3E].copy_from_slice(&0u16.to_be_bytes());
    elf[0x3E..0x40].copy_from_slice(&0u16.to_be_bytes());
}
