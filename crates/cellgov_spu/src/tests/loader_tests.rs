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
fn rejects_little_endian_elf() {
    // [CBE-Handbook p:393 s:14.2.2.1] SPE-ELF is defined as
    // ELFDATA2MSB, so EI_DATA=1 is not an SPE image.
    let mut s = SpuState::new();
    let mut data = [0u8; 52];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 1; // 32-bit (valid)
    data[5] = 1; // little-endian (rejected)
    assert_eq!(load_spu_elf(&data, &mut s), Err(LoadError::NotBigEndian));
}

/// ELF32-BE header with one PT_LOAD program header the caller
/// parameterises. Header fields: e_phoff at 28, e_phentsize at 42,
/// e_phnum at 44; the phdr's p_offset / p_vaddr / p_filesz / p_memsz
/// sit at +4 / +8 / +16 / +20.
fn mk_spu_elf(p_offset: u32, p_vaddr: u32, p_filesz: u32, p_memsz: u32, total: usize) -> Vec<u8> {
    let mut data = vec![0u8; total.max(52 + 32)];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 1;
    data[5] = 2;
    data[28..32].copy_from_slice(&52u32.to_be_bytes());
    data[42..44].copy_from_slice(&32u16.to_be_bytes());
    data[44..46].copy_from_slice(&1u16.to_be_bytes());
    let ph = 52usize;
    data[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    data[ph + 4..ph + 8].copy_from_slice(&p_offset.to_be_bytes());
    data[ph + 8..ph + 12].copy_from_slice(&p_vaddr.to_be_bytes());
    data[ph + 16..ph + 20].copy_from_slice(&p_filesz.to_be_bytes());
    data[ph + 20..ph + 24].copy_from_slice(&p_memsz.to_be_bytes());
    data
}

#[test]
fn rejects_segment_past_the_end_of_local_store() {
    // [CBE-Handbook p:64 s:3.1.1] Local Store is 256 KB; the last 16
    // bytes of LS cannot hold a 0x100-byte segment.
    let mut s = SpuState::new();
    let ls_len = s.ls.len() as u32;
    let data = mk_spu_elf(0x100, ls_len - 0x10, 0x100, 0x100, 0x400);
    assert_eq!(
        load_spu_elf(&data, &mut s),
        Err(LoadError::SegmentOutOfRange {
            vaddr: ls_len - 0x10,
            memsz: 0x100,
        })
    );
}

#[test]
fn rejects_segment_whose_file_image_runs_past_the_file() {
    let mut s = SpuState::new();
    // filesz 0x100 from offset 0x380 needs 0x480 bytes of file.
    let data = mk_spu_elf(0x380, 0, 0x100, 0x100, 0x400);
    assert_eq!(
        load_spu_elf(&data, &mut s),
        Err(LoadError::SegmentTruncated)
    );
}

#[test]
fn rejects_a_segment_claiming_more_file_bytes_than_memory_bytes() {
    let mut s = SpuState::new();
    // Placed so the memsz-derived bound passes and only the filesz
    // copy would run past local store: without the refusal this is a
    // slice-index panic, not an error.
    let ls_len = s.ls.len() as u32;
    let data = mk_spu_elf(0x100, ls_len - 4, 0x100, 0, 0x400);
    assert_eq!(
        load_spu_elf(&data, &mut s),
        Err(LoadError::SegmentFileszExceedsMemsz {
            filesz: 0x100,
            memsz: 0,
        })
    );
}

#[test]
fn the_bss_tail_beyond_filesz_is_zeroed_over_prior_local_store_contents() {
    let mut s = SpuState::new();
    // Dirty the whole destination first: a loader that only copied
    // filesz bytes would leave the tail carrying the old contents.
    for byte in &mut s.ls[0x200..0x300] {
        *byte = 0xA5;
    }
    let payload_off = 0x100usize;
    let mut data = mk_spu_elf(payload_off as u32, 0x200, 0x10, 0x80, 0x400);
    for byte in &mut data[payload_off..payload_off + 0x10] {
        *byte = 0x5C;
    }
    load_spu_elf(&data, &mut s).expect("load ok");
    assert!(
        s.ls[0x200..0x210].iter().all(|&b| b == 0x5C),
        "filesz bytes must land at p_vaddr"
    );
    assert!(
        s.ls[0x210..0x280].iter().all(|&b| b == 0),
        "the [filesz, memsz) tail must be zeroed"
    );
    assert!(
        s.ls[0x280..0x300].iter().all(|&b| b == 0xA5),
        "zeroing must stop at memsz"
    );
}

#[test]
#[cfg_attr(
    not(feature = "spu-microtests"),
    ignore = "needs the built micro-test corpus (tests/micro/*/build.sh); run with --features spu-microtests"
)]
fn loads_real_spu_elf() {
    let data = crate::tests::microtest_elf("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    let mut s = SpuState::new();
    load_spu_elf(&data, &mut s).unwrap();
    assert_eq!(s.pc, 0x160);
    let first_insn = u32::from_be_bytes([s.ls[0], s.ls[1], s.ls[2], s.ls[3]]);
    assert_eq!(first_insn, 0x7b00_0000);
}
