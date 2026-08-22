//! ELF64-BE program-header parser for the `disasm` subcommand.
//!
//! Owns the producer-side validation that lets [`super::stream::disassemble`]
//! skip per-byte bounds checks in its hot loop: PT_LOAD `[p_offset,
//! p_offset + p_filesz)` lies inside the file, and `p_vaddr + p_memsz`
//! fits in `u64`. Rejects non-PPC64 / non-ELFCLASS64 / non-big-endian
//! inputs at the entry point so downstream code can assume PS3 PPE
//! shape.

use cellgov_ps3_abi::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_EI_VERSION, ELF_E_MACHINE_OFFSET,
    ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET,
    ELF_PHOFF_OFFSET, ELF_PN_XNUM, EM_PPC64, EV_CURRENT, PHDR_P_FILESZ_OFFSET, PHDR_P_MEMSZ_OFFSET,
    PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};

/// Reason an ELF failed to parse.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum ElfError {
    #[error("not an ELF (file is {len} bytes; need >= {})", ELF_HEADER_SIZE)]
    TooSmall { len: usize },
    #[error("not an ELF (magic mismatch)")]
    BadMagic,
    #[error("ELF EI_CLASS=0x{ei_class:02x}; this tool only handles ELFCLASS64 (PS3 PPE objects)")]
    NotElf64 { ei_class: u8 },
    #[error("ELF EI_DATA=0x{ei_data:02x}; this tool only handles ELFDATA2MSB (PS3 PPE objects)")]
    NotBigEndian { ei_data: u8 },
    #[error("ELF EI_VERSION=0x{ei_version:02x}; only EV_CURRENT (1) is supported")]
    UnknownElfVersion { ei_version: u8 },
    #[error(
        "ELF e_machine={e_machine} (0x{e_machine:04x}); this tool only handles EM_PPC64 ({})",
        EM_PPC64
    )]
    NotPpc64 { e_machine: u16 },
    #[error(
        "ELF e_phentsize={phentsize} is smaller than Elf64_Phdr ({})",
        ELF_PHENTSIZE
    )]
    PhentsizeTooSmall { phentsize: u16 },
    #[error(
        "ELF e_phnum=0x{:04X} (PN_XNUM extension) is not supported by this tool",
        ELF_PN_XNUM
    )]
    PhdrCountExtended,
    #[error("ELF program-header arithmetic overflows: phoff=0x{phoff:x} phnum={phnum} phentsize={phentsize}")]
    PhdrTableOverflow {
        phoff: u64,
        phnum: u16,
        phentsize: u16,
    },
    #[error("ELF program-header table runs past file: phoff=0x{phoff:x} end=0x{phend:x} file_len=0x{file_len:x}")]
    PhdrOutOfFile {
        phoff: u64,
        phend: u64,
        file_len: u64,
    },
    #[error(
        "PT_LOAD #{idx} arithmetic overflows: p_offset=0x{p_offset:x} p_filesz=0x{p_filesz:x}"
    )]
    SegmentRangeOverflow {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
    },
    #[error("PT_LOAD #{idx} truncated: p_offset=0x{p_offset:x}+p_filesz=0x{p_filesz:x} runs past file_len=0x{file_len:x}")]
    SegmentTruncated {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
        file_len: u64,
    },
    #[error("PT_LOAD #{idx} vaddr-range overflows u64: p_vaddr=0x{p_vaddr:x} p_filesz=0x{p_filesz:x} p_memsz=0x{p_memsz:x}")]
    SegmentVaddrOverflow {
        idx: usize,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
    },
    #[error("PT_LOAD #{idx} has p_memsz=0x{p_memsz:x} < p_filesz=0x{p_filesz:x}; the ELF spec requires p_memsz >= p_filesz")]
    MemszLessThanFilesz {
        idx: usize,
        p_filesz: u64,
        p_memsz: u64,
    },
}

impl ElfError {
    pub(super) fn message(&self) -> String {
        self.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PtLoad {
    pub(super) vaddr: u64,
    pub(super) offset: u64,
    pub(super) filesz: u64,
    pub(super) memsz: u64,
}

/// Parse all PT_LOAD program headers out of an ELF64-BE blob.
///
/// Cross-module contract: `disassemble` relies on the validation here
/// (table extent, `e_phentsize`, `e_phnum != PN_XNUM`, every PT_LOAD's
/// `[p_offset, p_offset + p_filesz)` inside the file, and
/// `p_vaddr + p_memsz` not overflowing u64) to skip per-byte bounds
/// checks in its hot loop.
pub(super) fn parse_pt_loads(data: &[u8]) -> Result<Vec<PtLoad>, ElfError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(ElfError::TooSmall { len: data.len() });
    }
    if data[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if data[ELF_EI_CLASS] != ELFCLASS64 {
        return Err(ElfError::NotElf64 {
            ei_class: data[ELF_EI_CLASS],
        });
    }
    if data[ELF_EI_DATA] != ELFDATA2MSB {
        return Err(ElfError::NotBigEndian {
            ei_data: data[ELF_EI_DATA],
        });
    }
    if data[ELF_EI_VERSION] != EV_CURRENT {
        return Err(ElfError::UnknownElfVersion {
            ei_version: data[ELF_EI_VERSION],
        });
    }
    let e_machine = read_be_u16(data, ELF_E_MACHINE_OFFSET);
    if e_machine != EM_PPC64 {
        return Err(ElfError::NotPpc64 { e_machine });
    }

    let phoff = read_be_u64(data, ELF_PHOFF_OFFSET);
    let phentsize = read_be_u16(data, ELF_PHENTSIZE_OFFSET);
    let phnum = read_be_u16(data, ELF_PHNUM_OFFSET);

    if phnum == ELF_PN_XNUM {
        return Err(ElfError::PhdrCountExtended);
    }
    if (phentsize as usize) < ELF_PHENTSIZE {
        return Err(ElfError::PhentsizeTooSmall { phentsize });
    }

    let table_size =
        (phnum as u64)
            .checked_mul(phentsize as u64)
            .ok_or(ElfError::PhdrTableOverflow {
                phoff,
                phnum,
                phentsize,
            })?;
    let phend = phoff
        .checked_add(table_size)
        .ok_or(ElfError::PhdrTableOverflow {
            phoff,
            phnum,
            phentsize,
        })?;
    if phend > data.len() as u64 {
        return Err(ElfError::PhdrOutOfFile {
            phoff,
            phend,
            file_len: data.len() as u64,
        });
    }

    let mut out = Vec::new();
    for i in 0..phnum as usize {
        let base = phoff as usize + i * phentsize as usize;
        let p_type =
            u32::from_be_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = read_be_u64(data, base + PHDR_P_OFFSET_OFFSET);
        let p_vaddr = read_be_u64(data, base + PHDR_P_VADDR_OFFSET);
        let p_filesz = read_be_u64(data, base + PHDR_P_FILESZ_OFFSET);
        let p_memsz = read_be_u64(data, base + PHDR_P_MEMSZ_OFFSET);

        let seg_end_in_file =
            p_offset
                .checked_add(p_filesz)
                .ok_or(ElfError::SegmentRangeOverflow {
                    idx: i,
                    p_offset,
                    p_filesz,
                })?;
        if seg_end_in_file > data.len() as u64 {
            return Err(ElfError::SegmentTruncated {
                idx: i,
                p_offset,
                p_filesz,
                file_len: data.len() as u64,
            });
        }
        if p_memsz < p_filesz {
            return Err(ElfError::MemszLessThanFilesz {
                idx: i,
                p_filesz,
                p_memsz,
            });
        }
        // `select_segment` and the disassemble loop assume p_vaddr +
        // p_filesz (and p_memsz) fit in u64; rejecting here lets the
        // hot loop drop saturating_add.
        if p_vaddr.checked_add(p_filesz).is_none() || p_vaddr.checked_add(p_memsz).is_none() {
            return Err(ElfError::SegmentVaddrOverflow {
                idx: i,
                p_vaddr,
                p_filesz,
                p_memsz,
            });
        }
        out.push(PtLoad {
            vaddr: p_vaddr,
            offset: p_offset,
            filesz: p_filesz,
            memsz: p_memsz,
        });
    }
    Ok(out)
}

fn read_be_u16(data: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([data[off], data[off + 1]])
}

pub(super) fn read_be_u64(data: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ])
}

#[cfg(test)]
#[path = "tests/elf_tests.rs"]
mod tests;
