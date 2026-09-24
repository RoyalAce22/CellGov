//! Function-map inputs the loader fuzz sweep found.

use super::*;

/// Vaddr of the 0x40-byte executable text segment.
const TEXT_VADDR: u32 = 0x1_0000;
/// TOC word every descriptor in these images carries.
const TOC: u32 = 0x3_0000;
/// File offset of the data segment's first byte: the ELF header, two
/// program headers, then the 0x40 text bytes.
const DATA_FILE_OFF: usize = 64 + 2 * 56 + 0x40;
/// Bytes the file backs for the data segment.
const DATA_BACKED: usize = 0x40;

/// ELF64 ET_EXEC with an executable text segment at [`TEXT_VADDR`] and
/// a data segment at `data_vaddr`. The data segment's `p_filesz` and
/// `p_memsz` claim `claimed_filesz` bytes; the file holds
/// [`DATA_BACKED`].
fn exec_with_data_segment(data_vaddr: u64, claimed_filesz: u64) -> Vec<u8> {
    let phoff = 64usize;
    let phentsize = 56usize;
    let text_off = phoff + 2 * phentsize;
    let mut data = vec![0u8; DATA_FILE_OFF + DATA_BACKED];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[6] = 1; // EV_CURRENT
    data[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    data[16..18].copy_from_slice(&2u16.to_be_bytes());
    data[32..40].copy_from_slice(&(phoff as u64).to_be_bytes());
    data[54..56].copy_from_slice(&(phentsize as u16).to_be_bytes());
    data[56..58].copy_from_slice(&2u16.to_be_bytes());
    let text = phoff;
    data[text..text + 4].copy_from_slice(&1u32.to_be_bytes());
    data[text + 4..text + 8].copy_from_slice(&5u32.to_be_bytes());
    data[text + 8..text + 16].copy_from_slice(&(text_off as u64).to_be_bytes());
    data[text + 16..text + 24].copy_from_slice(&u64::from(TEXT_VADDR).to_be_bytes());
    data[text + 32..text + 40].copy_from_slice(&0x40u64.to_be_bytes());
    data[text + 40..text + 48].copy_from_slice(&0x40u64.to_be_bytes());
    let seg = phoff + phentsize;
    data[seg..seg + 4].copy_from_slice(&1u32.to_be_bytes());
    data[seg + 4..seg + 8].copy_from_slice(&6u32.to_be_bytes());
    data[seg + 8..seg + 16].copy_from_slice(&(DATA_FILE_OFF as u64).to_be_bytes());
    data[seg + 16..seg + 24].copy_from_slice(&data_vaddr.to_be_bytes());
    data[seg + 32..seg + 40].copy_from_slice(&claimed_filesz.to_be_bytes());
    data[seg + 40..seg + 48].copy_from_slice(&claimed_filesz.to_be_bytes());
    data
}

fn descriptor(code: u32, toc: u32) -> [u8; 8] {
    let mut d = [0u8; 8];
    d[0..4].copy_from_slice(&code.to_be_bytes());
    d[4..8].copy_from_slice(&toc.to_be_bytes());
    d
}

#[test]
fn the_descriptor_sweep_ends_where_the_file_does_not_where_filesz_claims() {
    let data = exec_with_data_segment(0x2_0000, 1 << 62);
    let map = build(&data).expect("a segment claim past the file is not a parse failure");
    assert!(map.functions.is_empty());
    assert!(!map.truncated);
}

#[test]
fn a_data_segment_at_the_top_of_the_address_space_is_swept_to_its_last_address() {
    // The segment ends at the top of the address space. The entry
    // descriptor sits at its first address; a second one sits in the
    // last slot whose end still fits in a u64. The final 8 bytes end
    // at 2^64, which no descriptor read accepts.
    const DATA_VADDR: u64 = u64::MAX - (DATA_BACKED as u64 - 1);
    let mut data = exec_with_data_segment(DATA_VADDR, DATA_BACKED as u64);
    data[24..32].copy_from_slice(&DATA_VADDR.to_be_bytes());
    data[DATA_FILE_OFF..DATA_FILE_OFF + 8].copy_from_slice(&descriptor(TEXT_VADDR, TOC));
    let last = DATA_FILE_OFF + DATA_BACKED - 0x10;
    data[last..last + 8].copy_from_slice(&descriptor(TEXT_VADDR + 0x20, TOC));

    let map = build(&data).expect("a segment ending at u64::MAX is not a parse failure");
    let starts: Vec<u32> = map.functions.iter().map(|s| s.start).collect();
    assert_eq!(starts, vec![TEXT_VADDR, TEXT_VADDR + 0x20]);
    assert_eq!(map.functions[0].origin, FunctionOrigin::EntryOpd);
    assert_eq!(map.functions[1].origin, FunctionOrigin::OpdScan);
    assert!(!map.truncated);
}

#[test]
fn a_data_segment_claiming_past_the_top_of_the_address_space_is_swept_only_to_the_top() {
    // The segment starts four bytes below the top of the address
    // space: only its first slot has an address, and that slot's
    // eight bytes end past the top.
    const DATA_VADDR: u64 = u64::MAX - 3;
    let mut data = exec_with_data_segment(DATA_VADDR, DATA_BACKED as u64);
    data[DATA_FILE_OFF..DATA_FILE_OFF + 8].copy_from_slice(&descriptor(TEXT_VADDR, TOC));

    let map = build(&data).expect("a segment past the top of the space is not a parse failure");
    assert!(map.functions.is_empty());
    assert!(!map.truncated);
}
