//! Embedded SPU ELF detection: what counts as an image, where it ends,
//! and what the scan passes over.

use super::*;
use cellgov_ps3_abi::format::elf::EM_PPC64;

/// A one-segment ELF32 big-endian image of `machine`, whose PT_LOAD
/// holds `code` at local-store 0x100 and whose entry is 0x100.
fn elf(machine: u16, code: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 52 + 32];
    out[..4].copy_from_slice(&ELF_MAGIC);
    out[4] = 1;
    out[5] = 2;
    out[18..20].copy_from_slice(&machine.to_be_bytes());
    out[24..28].copy_from_slice(&0x100u32.to_be_bytes());
    out[28..32].copy_from_slice(&52u32.to_be_bytes());
    out[42..44].copy_from_slice(&32u16.to_be_bytes());
    out[44..46].copy_from_slice(&1u16.to_be_bytes());
    let ph = 52;
    out[ph..ph + 4].copy_from_slice(&1u32.to_be_bytes());
    out[ph + 4..ph + 8].copy_from_slice(&84u32.to_be_bytes());
    out[ph + 8..ph + 12].copy_from_slice(&0x100u32.to_be_bytes());
    let len = code.len() as u32;
    out[ph + 16..ph + 20].copy_from_slice(&len.to_be_bytes());
    out[ph + 20..ph + 24].copy_from_slice(&len.to_be_bytes());
    out[ph + 24..ph + 28].copy_from_slice(&5u32.to_be_bytes());
    out.extend_from_slice(code);
    out
}

#[test]
fn a_file_that_is_an_spu_elf_is_one_image_at_offset_zero() {
    let image = elf(EM_SPU, &[0x40, 0x80, 0x00, 0x03]);
    let found = find_embedded_spu_elfs(&image);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].offset, 0);
    assert_eq!(found[0].elf.extent, image.len());
    assert_eq!(found[0].bytes(&image), &image[..]);
    assert_eq!(found[0].elf.segments[0].flags, 5);
}

#[test]
fn images_inside_other_bytes_are_found_in_order_with_their_extent() {
    let first = elf(EM_SPU, &[1, 2, 3, 4]);
    let second = elf(EM_SPU, &[5, 6, 7, 8, 9, 10, 11, 12]);
    let mut host = vec![0xAA; 37];
    host.extend_from_slice(&first);
    host.extend_from_slice(&[0x55; 11]);
    host.extend_from_slice(&second);
    host.extend_from_slice(&[0xCC; 5]);
    let found = find_embedded_spu_elfs(&host);
    let spans: Vec<(usize, usize)> = found.iter().map(|e| (e.offset, e.elf.extent)).collect();
    assert_eq!(
        spans,
        [(37, first.len()), (37 + first.len() + 11, second.len())]
    );
    assert_eq!(found[1].bytes(&host), &second[..]);
}

#[test]
fn a_ppu_elf_and_a_bare_magic_are_not_images() {
    let mut host = elf(EM_PPC64, &[0; 4]);
    host.extend_from_slice(&ELF_MAGIC);
    host.extend_from_slice(&[0; 60]);
    assert!(find_embedded_spu_elfs(&host).is_empty());
}

#[test]
fn a_segment_past_the_end_of_the_host_is_not_an_image() {
    let mut image = elf(EM_SPU, &[0; 16]);
    image.truncate(image.len() - 8);
    assert!(find_embedded_spu_elfs(&image).is_empty());
}

#[test]
fn a_section_header_table_inside_the_data_extends_the_image() {
    let mut image = elf(EM_SPU, &[0; 8]);
    let shoff = image.len() as u32;
    image[32..36].copy_from_slice(&shoff.to_be_bytes());
    image[46..48].copy_from_slice(&40u16.to_be_bytes());
    image[48..50].copy_from_slice(&2u16.to_be_bytes());
    image.extend_from_slice(&[0; 80]);
    let found = find_embedded_spu_elfs(&image);
    assert_eq!(found[0].elf.extent, image.len());
    // A table that would run past the data is a stripped image's stale
    // field and extends nothing.
    image.truncate(image.len() - 1);
    let found = find_embedded_spu_elfs(&image);
    assert_eq!(found[0].elf.extent, shoff as usize);
}
