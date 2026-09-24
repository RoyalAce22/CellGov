//! The one PT_LOAD reader's header refusals, the segment checks only
//! `checked_pt_loads` makes, and where an address's bytes come from.

use super::*;

/// One PT_LOAD: `(p_offset, p_vaddr, p_filesz, p_memsz)`.
type Load = (u64, u64, u64, u64);

/// A PPU ELF64 header and one program header per entry of `loads`, in a
/// file `len` bytes long.
fn elf(loads: &[Load], len: usize) -> Vec<u8> {
    let mut data = vec![0u8; len.max(64 + 56 * loads.len())];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[6] = EV_CURRENT;
    data[18..20].copy_from_slice(&EM_PPC64.to_be_bytes());
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&(loads.len() as u16).to_be_bytes());
    for (i, &(offset, vaddr, filesz, memsz)) in loads.iter().enumerate() {
        let base = 64 + 56 * i;
        data[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        data[base + 8..base + 16].copy_from_slice(&offset.to_be_bytes());
        data[base + 16..base + 24].copy_from_slice(&vaddr.to_be_bytes());
        data[base + 32..base + 40].copy_from_slice(&filesz.to_be_bytes());
        data[base + 40..base + 48].copy_from_slice(&memsz.to_be_bytes());
    }
    data
}

fn seg(vaddr: u64, file_offset: u64, filesz: u64, memsz: u64) -> LoadSegment {
    LoadSegment {
        index: 0,
        file_offset,
        vaddr,
        filesz,
        memsz,
        executable: true,
        writable: false,
        readable: true,
    }
}

#[test]
fn a_header_of_another_version_or_machine_is_refused_by_name() {
    let mut data = elf(&[(0x100, 0x1_0000, 4, 4)], 0x104);
    data[6] = 0;
    assert_eq!(
        read_pt_loads(&data),
        Err(LoadError::UnknownElfVersion { ei_version: 0 })
    );
    let mut data = elf(&[(0x100, 0x1_0000, 4, 4)], 0x104);
    // EM_X86_64: a well-formed ELF64-BE for another machine.
    data[18..20].copy_from_slice(&62u16.to_be_bytes());
    assert_eq!(
        read_pt_loads(&data),
        Err(LoadError::NotPpc64 { e_machine: 62 })
    );
}

#[test]
fn a_file_with_no_table_or_an_extended_count_is_refused_by_name() {
    let mut data = elf(&[], 64);
    // e_phnum=0 goes with e_phentsize=0; the count is read first, so
    // the absent table is named rather than the entry size.
    data[54..56].copy_from_slice(&0u16.to_be_bytes());
    assert_eq!(read_pt_loads(&data), Err(LoadError::NoProgramHeaders));
    let mut data = elf(&[], 64);
    data[56..58].copy_from_slice(&ELF_PN_XNUM.to_be_bytes());
    assert_eq!(read_pt_loads(&data), Err(LoadError::PhdrCountExtended));
}

#[test]
fn a_table_running_past_the_file_is_refused() {
    let mut data = elf(&[], 64);
    data[56..58].copy_from_slice(&1000u16.to_be_bytes());
    assert_eq!(read_pt_loads(&data), Err(LoadError::TooSmall));
}

#[test]
fn every_pt_load_is_read_zero_sized_ones_included_and_others_skipped() {
    let mut data = elf(
        &[(0x200, 0x1_0000, 4, 4), (0, 0x2_0000, 0, 0), (0, 0, 0, 0)],
        0x204,
    );
    // The third entry is PT_GNU_STACK.
    let third = 64 + 56 * 2;
    data[third..third + 4].copy_from_slice(&0x6474_E551u32.to_be_bytes());
    let all = read_pt_loads(&data).unwrap();
    assert_eq!(
        all.iter().map(|s| (s.index, s.vaddr)).collect::<Vec<_>>(),
        [(0, 0x1_0000), (1, 0x2_0000)]
    );
    assert_eq!(
        pt_load_segments(&data).unwrap().len(),
        1,
        "no zero-sized one"
    );
}

#[test]
fn segment_checks_belong_to_the_checked_read_alone() {
    // File bytes past the end of the file.
    let past_file = elf(&[(0x100, 0x1_0000, 0x10_0000, 0x10_0000)], 0x104);
    assert!(read_pt_loads(&past_file).is_ok());
    assert_eq!(
        checked_pt_loads(&past_file),
        Err(LoadError::SegmentTruncated {
            segment_index: 0,
            file_offset: 0x100,
            filesz: 0x10_0000,
            file_len: 0x104,
        })
    );
    // An end past the top of the address space.
    let past_top = elf(&[(0x100, u64::MAX, 4, 4)], 0x104);
    assert!(read_pt_loads(&past_top).is_ok());
    assert!(matches!(
        checked_pt_loads(&past_top),
        Err(LoadError::SegmentOutOfRange {
            segment_index: 0,
            ..
        })
    ));
    // More file bytes than memory bytes.
    let inverted = elf(&[(0x100, 0x1_0000, 16, 8)], 0x110);
    assert!(read_pt_loads(&inverted).is_ok());
    assert_eq!(
        checked_pt_loads(&inverted),
        Err(LoadError::SegmentFileszExceedsMemsz {
            segment_index: 0,
            filesz: 16,
            memsz: 8,
        })
    );
    // A file offset whose sum wraps.
    let wrapping = elf(&[(u64::MAX, 0x1_0000, 1, 1)], 0x100);
    assert!(matches!(
        checked_pt_loads(&wrapping),
        Err(LoadError::SegmentTruncated {
            segment_index: 0,
            ..
        })
    ));
}

#[test]
fn an_address_in_file_bytes_names_its_offset_and_how_many_segments_hold_it() {
    let big = seg(0x1_0000, 0x200, 0x1000, 0x1000);
    let small = seg(0x1_0000, 0x4000, 0x100, 0x100);
    assert_eq!(
        address_source(&[big, small], 0x1_0010, 4),
        AddressSource::FileBacked {
            segment: small,
            file_offset: 0x4010,
            overlapping: 2,
        }
    );
    assert_eq!(
        address_source(&[big, small], 0x1_0200, 4),
        AddressSource::FileBacked {
            segment: big,
            file_offset: 0x400,
            overlapping: 1,
        }
    );
}

#[test]
fn equal_sized_overlaps_break_on_the_lower_file_offset() {
    let a = seg(0x1_0000, 0x4000, 0x100, 0x100);
    let b = seg(0x1_0000, 0x2000, 0x100, 0x100);
    assert!(matches!(
        address_source(&[a, b], 0x1_0000, 1),
        AddressSource::FileBacked { segment, .. } if segment == b
    ));
}

#[test]
fn a_range_past_the_file_bytes_is_zero_fill_or_unmapped() {
    let bss = seg(0x1_0000, 0x200, 4, 0x100);
    assert_eq!(
        address_source(&[bss], 0x1_0004, 4),
        AddressSource::ZeroFill { segment: bss }
    );
    assert_eq!(
        address_source(&[bss], 0x1_0002, 4),
        AddressSource::ZeroFill { segment: bss },
        "a range straddling the file bytes' end is not file-backed"
    );
    assert_eq!(address_source(&[bss], 0x1_0100, 1), AddressSource::Unmapped);
    assert_eq!(address_source(&[bss], 0xFFFF, 1), AddressSource::Unmapped);
    assert_eq!(address_source(&[bss], 0x1_0004, 4).file_offset(), None);
}
