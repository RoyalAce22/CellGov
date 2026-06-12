//! SPU ELF loader validation and local-store image placement.

use super::*;
use crate::state::SpuState;

#[test]
fn rejects_too_small() {
    let mut s = SpuState::new();
    assert_eq!(load_spu_elf(&[0; 10], &mut s), Err(LoadError::TooSmall));
}

#[test]
fn rejects_bad_magic() {
    let mut s = SpuState::new();
    let mut data = [0u8; 52];
    data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    assert_eq!(load_spu_elf(&data, &mut s), Err(LoadError::BadMagic));
}

#[test]
fn rejects_64bit_elf() {
    let mut s = SpuState::new();
    let mut data = [0u8; 52];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2; // 64-bit
    data[5] = 2; // big-endian
    assert_eq!(load_spu_elf(&data, &mut s), Err(LoadError::Not32Bit));
}

#[test]
fn loads_real_spu_elf() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(path).unwrap();
    let mut s = SpuState::new();
    load_spu_elf(&data, &mut s).unwrap();
    assert_eq!(s.pc, 0x160);
    let first_insn = u32::from_be_bytes([s.ls[0], s.ls[1], s.ls[2], s.ls[3]]);
    assert_eq!(first_insn, 0x7b00_0000);
}
