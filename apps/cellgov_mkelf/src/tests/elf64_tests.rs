//! Big-endian PPC64 ELF construction: header fields, proc-param segment presence, and payload placement.

use super::*;

#[test]
fn build_produces_valid_elf_magic() {
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
    assert_eq!(&elf[0..4], b"\x7fELF");
}

#[test]
fn build_produces_big_endian_ppc64() {
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
    assert_eq!(elf[4], ELFCLASS64);
    assert_eq!(elf[5], ELFDATA2MSB);
    assert_eq!(&elf[18..20], &[0x00, 0x15]);
}

#[test]
fn build_entry_point_matches() {
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
    let entry = u64::from_be_bytes(elf[24..32].try_into().unwrap());
    assert_eq!(entry, 0x10000);
}

#[test]
fn build_without_proc_param_has_two_phdrs() {
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
    let phnum = u16::from_be_bytes(elf[56..58].try_into().unwrap());
    assert_eq!(phnum, 2);
}

#[test]
fn build_with_proc_param_has_three_phdrs() {
    let pp = proc_param(0x00360001);
    let mut data = vec![0u8; 8];
    let pp_offset = data.len() as u64;
    data.extend_from_slice(&pp);
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &data, Some(pp_offset));
    let phnum = u16::from_be_bytes(elf[56..58].try_into().unwrap());
    assert_eq!(phnum, 3);
}

#[test]
fn proc_param_has_correct_magic() {
    let pp = proc_param(0x00360001);
    let magic = u32::from_be_bytes(pp[4..8].try_into().unwrap());
    assert_eq!(magic, 0x13bcc5f6);
}

#[test]
#[should_panic(expected = "runs past the")]
fn a_proc_param_that_does_not_fit_the_data_segment_is_refused() {
    // 8 bytes of data, proc-param claimed at offset 0: the
    // PT_PROC_PARAM segment would declare 32 file-backed bytes that
    // the file does not contain.
    build(0x10000, 0x10000, &[0; 16], 0x20000, &[0u8; 8], Some(0));
}

#[test]
fn a_proc_param_ending_exactly_at_the_data_end_is_accepted() {
    let data = vec![0u8; PROC_PARAM_SIZE as usize];
    let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &data, Some(0));
    let phnum = u16::from_be_bytes(elf[56..58].try_into().unwrap());
    assert_eq!(phnum, 3);
    // p_filesz of the third phdr must stay inside the emitted file.
    let ph3 = 64 + 2 * 56;
    let p_offset = u64::from_be_bytes(elf[ph3 + 8..ph3 + 16].try_into().unwrap());
    let p_filesz = u64::from_be_bytes(elf[ph3 + 32..ph3 + 40].try_into().unwrap());
    assert_eq!(p_filesz, PROC_PARAM_SIZE);
    assert!(p_offset + p_filesz <= elf.len() as u64);
}

#[test]
#[should_panic(expected = "runs past the")]
fn a_proc_param_offset_that_overflows_the_file_extent_is_refused() {
    build(
        0x10000,
        0x10000,
        &[0; 16],
        0x20000,
        &[0u8; 64],
        Some(u64::MAX),
    );
}

#[test]
fn code_bytes_appear_in_output() {
    let code = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let elf = build(0x10000, 0x10000, &code, 0x20000, &[0; 4], None);
    assert!(elf.windows(4).any(|w| w == [0xDE, 0xAD, 0xBE, 0xEF]));
}

#[test]
fn data_bytes_appear_in_output() {
    let data = vec![0xCA, 0xFE, 0xBA, 0xBE];
    let elf = build(0x10000, 0x10000, &[0; 4], 0x20000, &data, None);
    assert!(elf.windows(4).any(|w| w == [0xCA, 0xFE, 0xBA, 0xBE]));
}
