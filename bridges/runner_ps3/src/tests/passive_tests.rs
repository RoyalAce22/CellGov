//! The passive-only scan: a relink that loads one of the numbers into
//! r11, or into a register a `mr r11` copies, or into an argument
//! register ahead of a call, or holds one in a data segment, is named
//! with the offset; any other number, the same
//! number used as a payload offset, a raw word in the code segment and
//! bytes outside every segment pass; and a file that is not a big-endian
//! ELF64 is refused rather than passed.

use std::path::Path;

use super::*;
use crate::memory_console::{ppu_elf, EXIT_ONLY};
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BL_OPCODE_LK, PPC_NOP};

const SC: u32 = 0x4400_0002;

/// `bl .+16`.
const BL: u32 = PPC_BL_OPCODE_LK | 0x10;

/// `mtctr r10`.
const MTCTR_R10: u32 = 0x7D49_03A6;

/// `bdnz .-8`: a `bc` with no link bit.
const BDNZ: u32 = 0x4200_FFF8;

/// `bcctrl 20, 0`: a call through the count register.
const BCCTRL: u32 = 0x4E80_0421;

/// `creqv 6, 6, 6`: an XL-form word that is not a branch.
const CREQV: u32 = 0x4CC6_3242;

/// The file offset of the first code word in a [`ppu_elf`].
const CODE_OFF: u64 = 120;

fn li(rd: u32, number: u64) -> u32 {
    LI | rd << 21 | number as u32
}

/// `mr ra, rs`, which is `or ra, rs, rs`.
fn mr(ra: u32, rs: u32) -> u32 {
    OR | rs << 21 | ra << 16 | rs << 11
}

fn scan(elf: &[u8]) -> Result<Option<(u64, u64)>, RunnerPs3Error> {
    first_passive_only_load(elf, Path::new("probe.elf"))
}

/// A [`ppu_elf`] holding `code` plus a second, read-write loadable
/// segment holding `data` right after it; returns the ELF and the data
/// segment's file offset.
fn with_data_segment(code: &[u32], data: &[u32]) -> (Vec<u8>, u64) {
    let mut elf = ppu_elf(code);
    // Grow the program-header count and move the code down by one header.
    let code_bytes = elf.split_off(CODE_OFF as usize);
    elf[0x38..0x3A].copy_from_slice(&2u16.to_be_bytes());
    let code_off = CODE_OFF + 56;
    elf[64 + 8..64 + 16].copy_from_slice(&code_off.to_be_bytes());
    let data_off = code_off + code_bytes.len() as u64;
    let data_len = 4 * data.len() as u64;
    let mut ph = vec![0u8; 56];
    ph[0..4].copy_from_slice(&1u32.to_be_bytes());
    ph[4..8].copy_from_slice(&6u32.to_be_bytes());
    ph[8..16].copy_from_slice(&data_off.to_be_bytes());
    ph[0x20..0x28].copy_from_slice(&data_len.to_be_bytes());
    ph[0x28..0x30].copy_from_slice(&data_len.to_be_bytes());
    elf.extend_from_slice(&ph);
    elf.extend_from_slice(&code_bytes);
    for w in data {
        elf.extend_from_slice(&w.to_be_bytes());
    }
    (elf, data_off)
}

#[test]
fn each_passive_only_number_loaded_into_r11_is_found_at_its_offset() {
    for number in PASSIVE_ONLY {
        let elf = ppu_elf(&[0x6000_0000, li(11, number), SC]);
        assert_eq!(
            scan(&elf).expect("a readable ELF"),
            Some((number, CODE_OFF + 4)),
            "{number}"
        );
    }
}

#[test]
fn a_number_loaded_into_another_register_for_an_out_of_line_wrapper_is_found() {
    let elf = ppu_elf(&[li(3, syscall::UART_SEND), PPC_NOP, BL]);
    assert_eq!(
        scan(&elf).expect("a readable ELF"),
        Some((syscall::UART_SEND, CODE_OFF))
    );
    let elf = ppu_elf(&[li(10, syscall::UART_RECEIVE), CREQV, BCCTRL]);
    assert_eq!(
        scan(&elf).expect("a readable ELF"),
        Some((syscall::UART_RECEIVE, CODE_OFF))
    );
}

#[test]
fn a_number_a_mr_copies_into_r11_is_found_from_any_register() {
    for rd in [3, 31] {
        let elf = ppu_elf(&[li(rd, syscall::UART_SEND), PPC_NOP, mr(11, rd), SC]);
        assert_eq!(
            scan(&elf).expect("a readable ELF"),
            Some((syscall::UART_SEND, CODE_OFF)),
            "r{rd}"
        );
    }
    let elf = ppu_elf(&[li(31, syscall::UART_SEND), mr(12, 31), SC]);
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn a_number_loaded_as_a_payload_offset_before_a_loop_branch_passes() {
    // The shape `put(46, ...)` compiles to: slot 46 sits at byte 368.
    let elf = ppu_elf(&[li(8, syscall::UART_RECEIVE), MTCTR_R10, PPC_NOP, BDNZ, BL]);
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn a_number_in_an_argument_register_with_no_call_in_the_window_passes() {
    let mut code = vec![li(3, syscall::UART_SEND)];
    code.extend([PPC_NOP; CALL_WINDOW]);
    code.push(BL);
    assert_eq!(scan(&ppu_elf(&code)).expect("a readable ELF"), None);
    let elf = ppu_elf(&[li(12, syscall::UART_SEND), BL]);
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn a_number_in_a_data_table_is_found() {
    let (elf, data_off) = with_data_segment(
        &EXIT_ONLY,
        &[
            0,
            syscall::TTY_WRITE as u32,
            0,
            syscall::UART_RECEIVE as u32,
        ],
    );
    assert_eq!(
        scan(&elf).expect("a readable ELF"),
        Some((syscall::UART_RECEIVE, data_off + 12))
    );
}

#[test]
fn get_params_and_the_harness_calls_pass() {
    let (elf, _) = with_data_segment(
        &[
            li(11, syscall::UART_GET_PARAMS),
            SC,
            li(11, syscall::TTY_WRITE),
            SC,
            EXIT_ONLY[0],
            SC,
        ],
        &[syscall::UART_GET_PARAMS as u32, syscall::TTY_WRITE as u32],
    );
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn a_raw_number_word_in_the_code_segment_passes() {
    let elf = ppu_elf(&[syscall::UART_RECEIVE as u32, EXIT_ONLY[0], SC]);
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn a_number_outside_every_loadable_segment_is_not_read() {
    let mut elf = ppu_elf(&EXIT_ONLY);
    elf.extend_from_slice(&li(11, syscall::UART_SEND).to_be_bytes());
    assert_eq!(scan(&elf).expect("a readable ELF"), None);
}

#[test]
fn an_empty_segment_whose_offset_lies_past_the_end_of_the_file_is_skipped() {
    let (mut elf, _) = with_data_segment(&[li(11, syscall::UART_SEND), SC], &[]);
    let bss_off = 64 + 56 + 8;
    elf[bss_off..bss_off + 8].copy_from_slice(&0x3_0000u64.to_be_bytes());
    assert!(elf.len() < 0x3_0000);
    assert_eq!(
        scan(&elf).expect("a readable ELF"),
        Some((syscall::UART_SEND, CODE_OFF + 56))
    );
}

#[test]
fn a_program_header_offset_at_the_top_of_the_address_space_is_refused_not_a_panic() {
    let mut elf = ppu_elf(&EXIT_ONLY);
    elf[0x20..0x28].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(
        scan(&elf),
        Err(RunnerPs3Error::UnscannableElf { .. })
    ));
}

#[test]
fn a_file_that_is_not_a_big_endian_elf64_is_refused() {
    let mut little = ppu_elf(&EXIT_ONLY);
    little[5] = 1;
    let mut short = ppu_elf(&EXIT_ONLY);
    short.truncate(100);
    for bad in [b"spu_fixed_value.elf".to_vec(), little, short] {
        assert!(matches!(
            scan(&bad),
            Err(RunnerPs3Error::UnscannableElf { elf }) if elf == Path::new("probe.elf")
        ));
    }
}
