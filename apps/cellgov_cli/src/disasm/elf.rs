#[derive(Debug, PartialEq, Eq)]
pub(super) enum ElfError {
    TooSmall {
        len: usize,
    },
    BadMagic,
    NotElf64 {
        ei_class: u8,
    },
    NotBigEndian {
        ei_data: u8,
    },
    UnknownElfVersion {
        ei_version: u8,
    },
    NotPpc64 {
        e_machine: u16,
    },
    PhentsizeTooSmall {
        phentsize: u16,
    },
    PhdrCountExtended,
    PhdrTableOverflow {
        phoff: u64,
        phnum: u16,
        phentsize: u16,
    },
    PhdrOutOfFile {
        phoff: u64,
        phend: u64,
        file_len: u64,
    },
    SegmentRangeOverflow {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
    },
    SegmentTruncated {
        idx: usize,
        p_offset: u64,
        p_filesz: u64,
        file_len: u64,
    },
    SegmentVaddrOverflow {
        idx: usize,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
    },
    MemszLessThanFilesz {
        idx: usize,
        p_filesz: u64,
        p_memsz: u64,
    },
}

impl ElfError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::TooSmall { len } => {
                format!("not an ELF (file is {len} bytes; need >= 64)")
            }
            Self::BadMagic => "not an ELF (magic mismatch)".to_string(),
            Self::NotElf64 { ei_class } => format!(
                "ELF EI_CLASS=0x{ei_class:02x}; this tool only handles ELFCLASS64 (PS3 PPE objects)"
            ),
            Self::NotBigEndian { ei_data } => format!(
                "ELF EI_DATA=0x{ei_data:02x}; this tool only handles ELFDATA2MSB (PS3 PPE objects)"
            ),
            Self::UnknownElfVersion { ei_version } => format!(
                "ELF EI_VERSION=0x{ei_version:02x}; only EV_CURRENT (1) is supported"
            ),
            Self::NotPpc64 { e_machine } => format!(
                "ELF e_machine={e_machine} (0x{e_machine:04x}); this tool only handles EM_PPC64 (21)"
            ),
            Self::PhentsizeTooSmall { phentsize } => format!(
                "ELF e_phentsize={phentsize} is smaller than Elf64_Phdr (56)"
            ),
            Self::PhdrCountExtended => {
                "ELF e_phnum=0xFFFF (PN_XNUM extension) is not supported by this tool".to_string()
            }
            Self::PhdrTableOverflow {
                phoff,
                phnum,
                phentsize,
            } => format!(
                "ELF program-header arithmetic overflows: phoff=0x{phoff:x} phnum={phnum} phentsize={phentsize}"
            ),
            Self::PhdrOutOfFile {
                phoff,
                phend,
                file_len,
            } => format!(
                "ELF program-header table runs past file: phoff=0x{phoff:x} end=0x{phend:x} file_len=0x{file_len:x}"
            ),
            Self::SegmentRangeOverflow {
                idx,
                p_offset,
                p_filesz,
            } => format!(
                "PT_LOAD #{idx} arithmetic overflows: p_offset=0x{p_offset:x} p_filesz=0x{p_filesz:x}"
            ),
            Self::SegmentTruncated {
                idx,
                p_offset,
                p_filesz,
                file_len,
            } => format!(
                "PT_LOAD #{idx} truncated: p_offset=0x{p_offset:x}+p_filesz=0x{p_filesz:x} runs past file_len=0x{file_len:x}"
            ),
            Self::SegmentVaddrOverflow {
                idx,
                p_vaddr,
                p_filesz,
                p_memsz,
            } => format!(
                "PT_LOAD #{idx} vaddr-range overflows u64: p_vaddr=0x{p_vaddr:x} p_filesz=0x{p_filesz:x} p_memsz=0x{p_memsz:x}"
            ),
            Self::MemszLessThanFilesz {
                idx,
                p_filesz,
                p_memsz,
            } => format!(
                "PT_LOAD #{idx} has p_memsz=0x{p_memsz:x} < p_filesz=0x{p_filesz:x}; the ELF spec requires p_memsz >= p_filesz"
            ),
        }
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
/// Validates EI_CLASS and EI_DATA, the program-header table extent,
/// `e_phentsize`, `e_phnum != PN_XNUM`, and that each PT_LOAD's
/// `[p_offset, p_offset + p_filesz)` lies entirely inside the file.
/// `disassemble` relies on those checks to skip per-byte bounds
/// validation in the hot loop.
pub(super) fn parse_pt_loads(data: &[u8]) -> Result<Vec<PtLoad>, ElfError> {
    if data.len() < 64 {
        return Err(ElfError::TooSmall { len: data.len() });
    }
    if &data[0..4] != b"\x7fELF" {
        return Err(ElfError::BadMagic);
    }
    if data[4] != 2 {
        return Err(ElfError::NotElf64 { ei_class: data[4] });
    }
    if data[5] != 2 {
        return Err(ElfError::NotBigEndian { ei_data: data[5] });
    }
    if data[6] != 1 {
        return Err(ElfError::UnknownElfVersion {
            ei_version: data[6],
        });
    }
    let e_machine = u16::from_be_bytes([data[18], data[19]]);
    // EM_PPC64 = 21 per the PowerPC ELF supplement.
    if e_machine != 21 {
        return Err(ElfError::NotPpc64 { e_machine });
    }

    let phoff = read_be_u64(data, 32);
    let phentsize = u16::from_be_bytes([data[54], data[55]]);
    let phnum = u16::from_be_bytes([data[56], data[57]]);

    if phnum == 0xFFFF {
        return Err(ElfError::PhdrCountExtended);
    }
    if (phentsize as usize) < 56 {
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
        if p_type != 1 {
            continue;
        }
        let p_offset = read_be_u64(data, base + 8);
        let p_vaddr = read_be_u64(data, base + 16);
        let p_filesz = read_be_u64(data, base + 32);
        let p_memsz = read_be_u64(data, base + 40);

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
        // Producer-side vaddr-range overflow check: select_segment and
        // the disassemble loop assume p_vaddr + p_filesz (and p_memsz)
        // fit in u64. Rejecting here lets the hot loop drop
        // saturating_add for plain +.
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
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn pt_loads_rejects_too_small() {
        assert_eq!(
            parse_pt_loads(&[0u8; 32]),
            Err(ElfError::TooSmall { len: 32 })
        );
    }

    #[test]
    fn pt_loads_rejects_bad_magic() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"NOPE");
        assert_eq!(parse_pt_loads(&data), Err(ElfError::BadMagic));
    }

    #[test]
    fn pt_loads_rejects_elfclass32() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        data[4] = 1; // ELFCLASS32
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::NotElf64 { ei_class: 1 })
        );
    }

    #[test]
    fn pt_loads_rejects_little_endian() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        data[5] = 1; // ELFDATA2LSB
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::NotBigEndian { ei_data: 1 })
        );
    }

    #[test]
    fn pt_loads_rejects_non_ppc64_machine() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        // EM_X86_64 = 62; well-formed ELF64-BE, just wrong machine.
        put_be_u16(&mut data, 18, 62);
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::NotPpc64 { e_machine: 62 })
        );
    }

    #[test]
    fn pt_loads_rejects_invalid_elf_version() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        data[6] = 0; // EI_VERSION = invalid
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::UnknownElfVersion { ei_version: 0 })
        );
    }

    #[test]
    fn pt_loads_rejects_pn_xnum() {
        let mut data = build_elf64_be(&[]);
        put_be_u16(&mut data, 56, 0xFFFF);
        assert_eq!(parse_pt_loads(&data), Err(ElfError::PhdrCountExtended));
    }

    #[test]
    fn pt_loads_rejects_phentsize_too_small() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        put_be_u16(&mut data, 54, 32);
        assert_eq!(
            parse_pt_loads(&data),
            Err(ElfError::PhentsizeTooSmall { phentsize: 32 })
        );
    }

    #[test]
    fn pt_loads_rejects_phdr_running_past_file() {
        let mut data = build_elf64_be(&[]);
        // Claim 1000 phdrs starting at offset 64; nowhere near enough file.
        put_be_u16(&mut data, 56, 1000);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::PhdrOutOfFile { .. }) => {}
            other => panic!("expected PhdrOutOfFile, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_segment_truncated_in_file() {
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        // Inflate p_filesz so seg_end_in_file > data.len()
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 32, 0x10_0000);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::SegmentTruncated { idx: 0, .. }) => {}
            other => panic!("expected SegmentTruncated, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_skips_non_pt_load_entries() {
        let mut spec = SegSpec::pt_load(0x200, 0x10000, NOP.to_vec());
        spec.p_type = 0x6474_E551; // PT_GNU_STACK
        let data = build_elf64_be(&[spec]);
        let segs = parse_pt_loads(&data).unwrap();
        assert!(segs.is_empty());
    }

    #[test]
    fn pt_loads_rejects_segment_vaddr_overflow() {
        // p_vaddr = u64::MAX, p_filesz = 1 -> p_vaddr + p_filesz overflows.
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 16, u64::MAX); // p_vaddr
                                                         // Leave p_filesz at 4 (the NOP we loaded) and p_memsz at 4.
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::SegmentVaddrOverflow { idx: 0, .. }) => {}
            other => panic!("expected SegmentVaddrOverflow, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_memsz_less_than_filesz() {
        // 16 bytes of content; poke p_memsz to 8 so memsz < filesz.
        let mut data = build_elf64_be(&[SegSpec::pt_load(
            0x200,
            0x10000,
            [NOP, NOP, NOP, NOP].concat(),
        )]);
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 40, 8); // p_memsz
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::MemszLessThanFilesz {
                idx: 0,
                p_filesz: 16,
                p_memsz: 8,
            }) => {}
            other => panic!("expected MemszLessThanFilesz, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_phdr_table_arithmetic_overflow() {
        // phoff=u64::MAX-10, phnum=1, phentsize=56 -> phoff+table_size overflows u64.
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        put_be_u64(&mut data, 32, u64::MAX - 10);
        put_be_u16(&mut data, 56, 1);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::PhdrTableOverflow { .. }) => {}
            other => panic!("expected PhdrTableOverflow, got {other:?}"),
        }
    }

    #[test]
    fn pt_loads_rejects_segment_range_overflow() {
        // Place a single PT_LOAD, then poke its p_offset to u64::MAX
        // and p_filesz to 1 so checked_add overflows.
        let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, NOP.to_vec())]);
        let phdr_base = 64usize;
        put_be_u64(&mut data, phdr_base + 8, u64::MAX);
        put_be_u64(&mut data, phdr_base + 32, 1);
        let result = parse_pt_loads(&data);
        match result {
            Err(ElfError::SegmentRangeOverflow { idx: 0, .. }) => {}
            other => panic!("expected SegmentRangeOverflow, got {other:?}"),
        }
    }
}
