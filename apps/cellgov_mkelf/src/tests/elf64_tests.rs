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
