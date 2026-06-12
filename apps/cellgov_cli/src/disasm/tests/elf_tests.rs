//! PT_LOAD parsing rejection paths for malformed or non-PS3 ELF input.

use super::*;
use crate::disasm::test_support::*;

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
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
    data[4] = 1; // ELFCLASS32
    assert_eq!(
        parse_pt_loads(&data),
        Err(ElfError::NotElf64 { ei_class: 1 })
    );
}

#[test]
fn pt_loads_rejects_little_endian() {
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
    data[5] = 1; // ELFDATA2LSB
    assert_eq!(
        parse_pt_loads(&data),
        Err(ElfError::NotBigEndian { ei_data: 1 })
    );
}

#[test]
fn pt_loads_rejects_non_ppc64_machine() {
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
    // EM_X86_64 = 62; well-formed ELF64-BE, just wrong machine.
    put_be_u16(&mut data, 18, 62);
    assert_eq!(
        parse_pt_loads(&data),
        Err(ElfError::NotPpc64 { e_machine: 62 })
    );
}

#[test]
fn pt_loads_rejects_invalid_elf_version() {
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
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
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
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
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
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
    let mut spec = SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec());
    spec.p_type = 0x6474_E551; // PT_GNU_STACK
    let data = build_elf64_be(&[spec]);
    let segs = parse_pt_loads(&data).unwrap();
    assert!(segs.is_empty());
}

#[test]
fn pt_loads_rejects_segment_vaddr_overflow() {
    // p_vaddr = u64::MAX, p_filesz = 1 -> p_vaddr + p_filesz overflows.
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
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
        [PPC_NOP_BYTES, PPC_NOP_BYTES, PPC_NOP_BYTES, PPC_NOP_BYTES].concat(),
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
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
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
    let mut data = build_elf64_be(&[SegSpec::pt_load(0x200, 0x10000, PPC_NOP_BYTES.to_vec())]);
    let phdr_base = 64usize;
    put_be_u64(&mut data, phdr_base + 8, u64::MAX);
    put_be_u64(&mut data, phdr_base + 32, 1);
    let result = parse_pt_loads(&data);
    match result {
        Err(ElfError::SegmentRangeOverflow { idx: 0, .. }) => {}
        other => panic!("expected SegmentRangeOverflow, got {other:?}"),
    }
}
