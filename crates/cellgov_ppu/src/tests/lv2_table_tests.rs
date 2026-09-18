//! Synthetic kernel shapes for LV2 dispatch-table discovery.

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
const HANDLER: u64 = BASE + 0x1000;
const TABLE: u64 = DATA_VADDR;
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

fn kernel_with_table(include_table: bool) -> Vec<u8> {
    let mut elf = vec![0u8; DATA_FILE + DATA_SIZE];
    elf[..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    write_u64(&mut elf, ELF_PHOFF_OFFSET, ELF_HEADER_SIZE as u64);
    write_u16(&mut elf, ELF_PHENTSIZE_OFFSET, ELF_PHENTSIZE as u16);
    write_u16(&mut elf, ELF_PHNUM_OFFSET, 2);
    program_header(&mut elf, 0, PF_R | PF_X, EXEC_FILE, BASE, EXEC_SIZE);
    program_header(&mut elf, 1, PF_R | PF_W, DATA_FILE, DATA_VADDR, DATA_SIZE);

    // [PPC-Book3 p:73 s:5.5.13] The System Call interrupt resumes at EA 0xC00.
    let vector = EXEC_FILE + 0xc00;
    for (index, word) in [
        0x3c80_8000, // lis r4, 0x8000
        0x6084_0000, // ori r4, r4, 0
        0x7884_07c6, // sldi r4, r4, 32
        0x6484_0000, // oris r4, r4, 0
        0x6084_1000, // ori r4, r4, 0x1000
    ]
    .into_iter()
    .enumerate()
    {
        write_u32(&mut elf, vector + index * 4, word);
    }
    for (index, word) in [
        0x3c80_8000, // lis r4, 0x8000
        0x6084_0000, // ori r4, r4, 0
        0x7884_07c6, // sldi r4, r4, 32
        0x6484_0000, // oris r4, r4, 0
        0x6084_4000, // ori r4, r4, 0x4000 (non-executable data)
    ]
    .into_iter()
    .enumerate()
    {
        write_u32(&mut elf, vector + (index + 8) * 4, word);
    }

    let handler = EXEC_FILE + 0x1000;
    for (index, word) in [
        0x282b_0004, // cmpldi r11, 4
        0x41e0_0008, // blt +8
        0x3960_0000, // li r11, 0
        0x796b_1f24, // sldi r11, r11, 3
        0x7dab_6a14, // add r13, r11, r13
        0xe9ad_0000, // ld r13, 0(r13)
        0xe9ad_0000, // ld r13, 0(r13)
        0xf821_ff91, // stdu r1, -112(r1)
        0x7da8_03a6, // mtlr r13
        0x4e80_0021, // blrl
    ]
    .into_iter()
    .enumerate()
    {
        write_u32(&mut elf, handler + index * 4, word);
    }

    if include_table {
        for index in 0..4usize {
            // The kernel table uses three consecutive big-endian u64
            // words for code, TOC, and environment.
            let descriptor = DATA_VADDR + 0x100 + (index * 24) as u64;
            write_u64(
                &mut elf,
                DATA_FILE + index * 8,
                if index == 3 {
                    DATA_VADDR + 0x100
                } else {
                    descriptor
                },
            );
            let descriptor_file = DATA_FILE + 0x100 + index * 24;
            let code = BASE + 0x1400 + (index * 0x10) as u64;
            write_u64(&mut elf, descriptor_file, code);
            write_u64(&mut elf, descriptor_file + 8, TOC);
            write_u64(&mut elf, descriptor_file + 16, 0);
            let code_file = EXEC_FILE + 0x1400 + index * 0x10;
            if index == 0 {
                for (word_index, word) in [0x3c60_8001, 0x6063_0003, 0x4e80_0020]
                    .into_iter()
                    .enumerate()
                {
                    write_u32(&mut elf, code_file + word_index * 4, word);
                }
            } else {
                write_u32(&mut elf, code_file, 0x4e80_0020);
            }
        }
    }
    elf
}

#[test]
fn a_vector_and_unique_descriptor_array_produce_high_confidence() {
    let found = discover(&kernel_with_table(true)).expect("discover table");
    assert_eq!(found.method, Lv2DiscoveryMethod::ScVectorDescriptorArray);
    assert_eq!(found.confidence, Lv2DiscoveryConfidence::High);
    assert_eq!(found.vector_vaddr, BASE + 0xc00);
    assert_eq!(found.handler_vaddr, HANDLER);
    assert_eq!(found.table_vaddr, TABLE);
    assert_eq!(found.table_file_offset, DATA_FILE as u64);
    assert_eq!(found.entry_count, 4);
    assert_eq!(found.entry_width, 8);
    assert_eq!(
        found.entry_format,
        Lv2TableEntryFormat::Ppc64DescriptorPointer
    );
    assert_eq!(found.toc, TOC);
    assert_eq!(found.evidence.vector_targets, 1);
    assert_eq!(found.evidence.handler_matches, 1);
    assert_eq!(found.evidence.table_candidates, 1);
    assert_eq!(found.evidence.descriptor_entries, 4);
    assert_eq!(found.evidence.unique_descriptors, 3);
    assert_eq!(found.evidence.entry_zero_references, 2);
    assert!(found.evidence.last_entry_is_entry_zero);
    assert_eq!(found.evidence.zero_environments, 4);
    assert!(found.evidence.consistent_toc);
    assert_eq!(found.evidence.post_table_zero, Some(true));
    assert_eq!(found.evidence.entry_zero_return, Some(0x8001_0003));
}

#[test]
fn a_kernel_with_the_handler_but_no_table_refuses_by_shape() {
    assert!(matches!(
        discover(&kernel_with_table(false)),
        Err(Lv2TableDiscoveryError::NoTableCandidate {
            entry_count: 4,
            entry_width: 8
        })
    ));
}

#[test]
fn two_equally_valid_arrays_refuse_ambiguity() {
    let mut elf = kernel_with_table(true);
    let table = elf[DATA_FILE..DATA_FILE + 32].to_vec();
    elf[DATA_FILE + 0x800..DATA_FILE + 0x820].copy_from_slice(&table);
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::AmbiguousTable { count: 2 })
    ));
}

#[test]
fn a_nonzero_word_after_the_table_is_reported_not_assumed_away() {
    let mut elf = kernel_with_table(true);
    write_u64(&mut elf, DATA_FILE + 32, BASE + 0x1500);
    let found = discover(&elf).expect("discover table before adjacent data");
    assert_eq!(found.evidence.post_table_zero, Some(false));
}

#[test]
fn a_table_at_the_file_backed_segment_end_has_unknown_post_data() {
    let mut elf = kernel_with_table(true);
    let table = elf[DATA_FILE..DATA_FILE + 32].to_vec();
    elf[DATA_FILE..DATA_FILE + 32].fill(0);
    let end_table = DATA_FILE + DATA_SIZE - 32;
    elf[end_table..end_table + 32].copy_from_slice(&table);
    let found = discover(&elf).expect("discover table at segment end");
    assert_eq!(found.table_vaddr, DATA_VADDR + DATA_SIZE as u64 - 32);
    assert_eq!(found.evidence.post_table_zero, None);
}

#[test]
fn a_vector_literal_without_the_sldi_mask_is_not_a_target() {
    let mut elf = kernel_with_table(true);
    let vector_shift = EXEC_FILE + 0xc00 + 2 * 4;
    write_u32(&mut elf, vector_shift, 0x7884_0786); // rldicr r4, r4, 32, 30
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn a_handler_branch_on_another_condition_is_not_an_index_bound() {
    let mut elf = kernel_with_table(true);
    write_u32(&mut elf, EXEC_FILE + 0x1000 + 4, 0x41e2_0008); // branch on CR0 EQ
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn a_handler_masked_rotate_is_not_an_entry_width() {
    let mut elf = kernel_with_table(true);
    write_u32(&mut elf, EXEC_FILE + 0x1000 + 3 * 4, 0x796b_1ee4); // rldicr r11, r11, 3, 59
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn a_conditional_link_register_call_is_not_a_dispatch_call() {
    let mut elf = kernel_with_table(true);
    write_u32(&mut elf, EXEC_FILE + 0x1000 + 9 * 4, 0x4d80_0021); // bclrl 12, 0
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn two_index_sequences_in_one_handler_refuse_ambiguity() {
    let mut elf = kernel_with_table(true);
    let handler = EXEC_FILE + 0x1000;
    let second = handler + 0x100;
    let sequence = elf[handler..handler + 40].to_vec();
    elf[second..second + 40].copy_from_slice(&sequence);
    write_u32(&mut elf, second, 0x282b_0005); // cmpldi r11, 5
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::AmbiguousHandler { count: 2 })
    ));
}

#[test]
fn a_conditional_entry_zero_return_is_not_a_constant_leaf() {
    let mut elf = kernel_with_table(true);
    write_u32(&mut elf, EXEC_FILE + 0x1400 + 8, 0x4d80_0020);
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::NoTableCandidate { .. })
    ));
}

#[test]
fn an_index_shape_after_an_unconditional_return_is_unreachable() {
    let mut elf = kernel_with_table(true);
    let handler = EXEC_FILE + 0x1000;
    let sequence = elf[handler..handler + 40].to_vec();
    elf[handler + 0x100..handler + 0x128].copy_from_slice(&sequence);
    write_u32(&mut elf, handler, 0x4e80_0020); // blr
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn a_target_register_clobber_before_mtlr_is_not_a_dispatch_shape() {
    let mut elf = kernel_with_table(true);
    let gap = EXEC_FILE + 0x1000 + 7 * 4;
    write_u32(&mut elf, gap, 0x39a0_0000); // li r13, 0
    assert!(matches!(
        discover(&elf),
        Err(Lv2TableDiscoveryError::HandlerNotFound)
    ));
}

#[test]
fn an_overflow_in_one_segment_does_not_hide_a_later_mapping() {
    let address = u64::MAX - 3;
    let segments = [
        LoadSegment {
            index: 0,
            file_offset: 0,
            vaddr: 0,
            filesz: u64::MAX,
            memsz: u64::MAX,
            executable: false,
            writable: false,
            readable: true,
        },
        LoadSegment {
            index: 1,
            file_offset: 0x80,
            vaddr: address,
            filesz: 8,
            memsz: 8,
            executable: false,
            writable: false,
            readable: true,
        },
    ];
    assert_eq!(file_offset_at(&segments, address, 8), Some(0x80));
}
