use super::*;
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE,
    ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, PF_R, PF_W, PF_X,
    PHDR_P_FILESZ_OFFSET, PHDR_P_MEMSZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};

const BASE: u64 = 0x8000_0000_0000_0000;
const EXEC_FILE: usize = 0x1000;
const EXEC_SIZE: usize = 0x2000;
const DATA_FILE: usize = 0x3000;
const DATA_VADDR: u64 = BASE + 0x4000;
const DATA_SIZE: usize = 0x1000;
const TABLE_ENTRIES: usize = 16;
const TOC: u64 = DATA_VADDR + 0xf00;

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn program_header(
    elf: &mut [u8],
    index: usize,
    flags: u32,
    file_offset: usize,
    vaddr: u64,
    size: usize,
) {
    let base = ELF_HEADER_SIZE + index * ELF_PHENTSIZE;
    write_u32(elf, base, PT_LOAD);
    write_u32(elf, base + 4, flags);
    write_u64(elf, base + PHDR_P_OFFSET_OFFSET, file_offset as u64);
    write_u64(elf, base + PHDR_P_VADDR_OFFSET, vaddr);
    write_u64(elf, base + PHDR_P_FILESZ_OFFSET, size as u64);
    write_u64(elf, base + PHDR_P_MEMSZ_OFFSET, size as u64);
}

fn descriptor_address(index: usize) -> u64 {
    DATA_VADDR + 0x200 + (index * 24) as u64
}

fn kernel_with_entries(entries: &[Option<usize>; TABLE_ENTRIES]) -> Vec<u8> {
    let mut elf = vec![0u8; DATA_FILE + DATA_SIZE];
    elf[..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    write_u64(&mut elf, ELF_PHOFF_OFFSET, ELF_HEADER_SIZE as u64);
    write_u16(&mut elf, ELF_PHENTSIZE_OFFSET, ELF_PHENTSIZE as u16);
    write_u16(&mut elf, ELF_PHNUM_OFFSET, 2);
    program_header(&mut elf, 0, PF_R | PF_X, EXEC_FILE, BASE, EXEC_SIZE);
    program_header(&mut elf, 1, PF_R | PF_W, DATA_FILE, DATA_VADDR, DATA_SIZE);

    let vector = EXEC_FILE + 0xc00;
    for (index, word) in [
        0x3c80_8000,
        0x6084_0000,
        0x7884_07c6,
        0x6484_0000,
        0x6084_1000,
    ]
    .into_iter()
    .enumerate()
    {
        write_u32(&mut elf, vector + index * 4, word);
    }

    let handler = EXEC_FILE + 0x1000;
    for (index, word) in [
        0x282b_0010, // cmpldi r11, 16
        0x41e0_0008,
        0x3960_0000,
        0x796b_1f24,
        0x7dab_6a14,
        0xe9ad_0000,
        0xe9ad_0000,
        0xf821_ff91,
        0x7da8_03a6,
        0x4e80_0021,
    ]
    .into_iter()
    .enumerate()
    {
        write_u32(&mut elf, handler + index * 4, word);
    }

    for (ordinal, descriptor) in entries.iter().enumerate() {
        write_u64(
            &mut elf,
            DATA_FILE + ordinal * 8,
            descriptor.map_or(0, descriptor_address),
        );
    }
    for index in 0..4usize {
        let descriptor_file = DATA_FILE + 0x200 + index * 24;
        let code = BASE + 0x1500 + (index * 0x20) as u64;
        write_u64(&mut elf, descriptor_file, code);
        write_u64(&mut elf, descriptor_file + 8, TOC);
        write_u64(&mut elf, descriptor_file + 16, 0);
        let code_file = EXEC_FILE + 0x1500 + index * 0x20;
        match index {
            0 => write_constant_return(&mut elf, code_file, 0x8001_0003),
            2 => write_constant_return(&mut elf, code_file, 0x8001_0002),
            _ => write_u32(&mut elf, code_file, 0x4e80_0020),
        }
    }
    elf
}

fn write_constant_return(elf: &mut [u8], offset: usize, value: u32) {
    write_u32(elf, offset, 0x3c60_0000 | (value >> 16));
    write_u32(elf, offset + 4, 0x6063_0000 | (value & 0xffff));
    write_u32(elf, offset + 8, 0x4e80_0020);
}

#[test]
fn dominant_constant_error_target_classifies_every_ordinal() {
    let elf = kernel_with_entries(&[
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(2),
        Some(3),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(3),
        Some(0),
        Some(2),
        Some(0),
    ]);
    let classified = classify(&elf).expect("classify table");
    assert_eq!(classified.primary_stub.descriptor, descriptor_address(0));
    assert_eq!(classified.primary_stub.errno, 0x8001_0003);
    assert_eq!(classified.primary_stub.errno_symbol, "CELL_ENOSYS");
    assert_eq!(classified.primary_stub.references, 10);
    assert_eq!(classified.stub_targets.len(), 2);
    assert_eq!(classified.implemented, 4);
    assert_eq!(classified.stub, 12);
    assert_eq!(classified.absent, 0);
    assert_eq!(
        classified
            .ordinals
            .iter()
            .map(|entry| (entry.ordinal, entry.class))
            .collect::<Vec<_>>(),
        vec![
            (0, Lv2OrdinalClass::Stub),
            (1, Lv2OrdinalClass::Stub),
            (2, Lv2OrdinalClass::Implemented),
            (3, Lv2OrdinalClass::Stub),
            (4, Lv2OrdinalClass::Stub),
            (5, Lv2OrdinalClass::Implemented),
            (6, Lv2OrdinalClass::Stub),
            (7, Lv2OrdinalClass::Stub),
            (8, Lv2OrdinalClass::Stub),
            (9, Lv2OrdinalClass::Stub),
            (10, Lv2OrdinalClass::Implemented),
            (11, Lv2OrdinalClass::Stub),
            (12, Lv2OrdinalClass::Implemented),
            (13, Lv2OrdinalClass::Stub),
            (14, Lv2OrdinalClass::Stub),
            (15, Lv2OrdinalClass::Stub),
        ]
    );
    assert_eq!(classified.evidence.runner_up_references, 2);
}

#[test]
fn zero_table_entry_is_absent() {
    let elf = kernel_with_entries(&[
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(2),
        None,
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(3),
        Some(0),
        Some(2),
        Some(0),
    ]);
    let classified = classify(&elf).expect("classify table with absent slot");
    assert_eq!(classified.ordinals[5].class, Lv2OrdinalClass::Absent);
    assert_eq!(classified.ordinals[5].descriptor, None);
    assert_eq!(classified.absent, 1);
}

#[test]
fn absent_ordinals_do_not_raise_the_descriptor_mode_threshold() {
    let elf = kernel_with_entries(&[
        Some(0),
        Some(0),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(0),
    ]);
    let classified = classify(&elf).expect("classify sparse table");
    assert_eq!(classified.evidence.mode_references, 3);
    assert_eq!(classified.evidence.minimum_mode_references, 1);
    assert_eq!(classified.stub, 3);
    assert_eq!(classified.absent, 13);
}

#[test]
fn short_constant_return_candidate_is_not_a_panic() {
    let mut elf = kernel_with_entries(&[
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(2),
        Some(3),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(3),
        Some(0),
        Some(2),
        Some(0),
    ]);
    let short_leaf = BASE + EXEC_SIZE as u64 - 4;
    write_u64(&mut elf, DATA_FILE + 0x200 + 24, short_leaf);
    write_u32(&mut elf, EXEC_FILE + EXEC_SIZE - 4, 0x3c60_8001);
    let classified = classify(&elf).expect("classify table with short code tail");
    assert_eq!(classified.ordinals[2].class, Lv2OrdinalClass::Implemented);
}

#[test]
fn tied_histogram_refuses_to_guess() {
    let elf = kernel_with_entries(&[
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(1),
        Some(2),
        Some(3),
        Some(0),
    ]);
    assert!(matches!(
        classify(&elf),
        Err(Lv2StubClassificationError::NoClearMode {
            top: 4,
            runner_up: 4,
            minimum: 4,
            factor: 4,
        })
    ));
}

#[test]
fn dominant_nonconstant_target_refuses_to_be_a_stub() {
    let elf = kernel_with_entries(&[
        Some(0),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(3),
        Some(2),
        Some(1),
        Some(0),
    ]);
    assert!(matches!(
        classify(&elf),
        Err(Lv2StubClassificationError::ModeNotConstantError { descriptor })
            if descriptor == descriptor_address(3)
    ));
}
