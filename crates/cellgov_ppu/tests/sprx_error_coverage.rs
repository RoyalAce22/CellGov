//! Locking tests for `PrxLoadError` variants that the per-type
//! reloc fixture files don't reach: ADDR32 patch-offset alignment,
//! ADDR32 `value >> 32` overflow, `SegmentOverlap`, and the two
//! flavors of `SegmentSizeOverflow`.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::graph::module_id_from_name;
use cellgov_ppu::sprx::{
    load_prx, ParsedPrx, PrxLoadError, PrxRelocation, PrxSegment, RelocMisalignedKind,
    R_PPC64_ADDR32, R_PPC64_ADDR64,
};

const TEXT_VADDR: u64 = 0x0000;
const DATA_VADDR: u64 = 0x1_0000;
const SEG_SIZE: u64 = 0x1_0000;
const BASE: u64 = 0x4000_0000;

fn fresh_memory() -> GuestMemory {
    GuestMemory::from_regions(vec![Region::new(
        BASE,
        (2 * SEG_SIZE) as usize,
        "main",
        PageSize::Page64K,
    )])
    .expect("memory")
}

fn parsed_with_relocs(relocs: Vec<PrxRelocation>) -> ParsedPrx {
    ParsedPrx {
        name: "test_coverage".to_string(),
        module_id: module_id_from_name("test_coverage"),
        toc: 0,
        text: PrxSegment {
            vaddr: TEXT_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        data: PrxSegment {
            vaddr: DATA_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        exports: vec![],
        relocations: relocs,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn addr32_patch_offset_misaligned_rejects_with_patchoffset_kind() {
    // The PatchOffset arm fires for any rtype whose offset is not
    // aligned to its write width. ADDR32 wants 4-byte alignment;
    // 0x101 is misaligned by 1.
    let mut mem = fresh_memory();
    let parsed = parsed_with_relocs(vec![PrxRelocation {
        offset: 0x101,
        rtype: R_PPC64_ADDR32,
        sym: 0,
        addend: 0,
    }]);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocMisaligned { rtype, kind, value } => {
            assert_eq!(rtype, R_PPC64_ADDR32);
            assert_eq!(kind, RelocMisalignedKind::PatchOffset);
            assert_eq!(value, 0x101);
        }
        other => panic!("expected RelocMisaligned, got {other:?}"),
    }
}

#[test]
fn addr32_value_above_32_bits_rejects_with_overflow() {
    // value = BASE + 0 + 0x1_0000_0000 = 0x1_4000_0000 -- bit 32
    // is non-zero, so the `value >> 32 != 0` guard fires. Without
    // it, `as u32` would silently store 0x4000_0000 and the patch
    // would land at the truncated address with no signal.
    let mut mem = fresh_memory();
    let parsed = parsed_with_relocs(vec![PrxRelocation {
        offset: 0x0,
        rtype: R_PPC64_ADDR32,
        sym: 0,
        addend: 0x1_0000_0000_i64,
    }]);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocOverflow { rtype, delta } => {
            assert_eq!(rtype, R_PPC64_ADDR32);
            assert_eq!(delta, 0x1_4000_0000_i64);
        }
        other => panic!("expected RelocOverflow, got {other:?}"),
    }
}

#[test]
fn reloc_offset_out_of_segment_spill_window_rejects() {
    // Defense-in-depth path: an aligned offset that fits within
    // seg_size but whose write spills past it. Only reachable
    // with non-write-width-aligned memsz (real PRX segments are
    // page-aligned, so this is synthetic). ADDR64 width = 8;
    // memsz = 0xFFFE is 2-aligned but not 8-aligned. offset =
    // 0xFFF8 is 8-aligned and < memsz, but offset + 8 = 0x10000
    // > memsz. The aligned-offset case at offset == seg_size is
    // covered by reloc_addr64::addr64_at_segment_size_offset_*;
    // this is the other branch of RelocOffsetOutOfSegment.
    let mut mem = fresh_memory();
    let synthetic_memsz: u64 = 0xFFFE;
    let parsed = ParsedPrx {
        name: "test_spill".to_string(),
        module_id: module_id_from_name("test_spill"),
        toc: 0,
        text: PrxSegment {
            vaddr: TEXT_VADDR,
            filesz: synthetic_memsz,
            memsz: synthetic_memsz,
            data: vec![0u8; synthetic_memsz as usize],
        },
        data: PrxSegment {
            vaddr: DATA_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        exports: vec![],
        relocations: vec![PrxRelocation {
            offset: 0xFFF8,
            rtype: R_PPC64_ADDR64,
            sym: 0,
            addend: 0,
        }],
        module_start: None,
        module_stop: None,
    };
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocOffsetOutOfSegment {
            rtype,
            offset,
            seg_size,
        } => {
            assert_eq!(rtype, R_PPC64_ADDR64);
            assert_eq!(offset, 0xFFF8);
            assert_eq!(seg_size, 0xFFFE);
        }
        other => panic!("expected RelocOffsetOutOfSegment, got {other:?}"),
    }
}

#[test]
fn segment_overlap_rejects_when_text_filesz_overlaps_data_vaddr() {
    // Place text at vaddr 0 with filesz extending past data.vaddr.
    // text.filesz = 0x2_0000 reaches into data's start (0x1_0000),
    // so the symmetric overlap check fires.
    let mut mem = GuestMemory::from_regions(vec![Region::new(
        BASE,
        (4 * SEG_SIZE) as usize,
        "main",
        PageSize::Page64K,
    )])
    .expect("memory");
    let parsed = ParsedPrx {
        name: "test_overlap".to_string(),
        module_id: module_id_from_name("test_overlap"),
        toc: 0,
        text: PrxSegment {
            vaddr: 0,
            filesz: 2 * SEG_SIZE,
            memsz: 2 * SEG_SIZE,
            data: vec![0u8; (2 * SEG_SIZE) as usize],
        },
        data: PrxSegment {
            vaddr: SEG_SIZE, // overlaps text
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        exports: vec![],
        relocations: vec![],
        module_start: None,
        module_stop: None,
    };
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::SegmentOverlap {
            first_end,
            second_start,
        } => {
            // text content ends at BASE + 0x2_0000; data starts at
            // BASE + 0x1_0000.
            assert_eq!(first_end, BASE + 2 * SEG_SIZE);
            assert_eq!(second_start, BASE + SEG_SIZE);
        }
        other => panic!("expected SegmentOverlap, got {other:?}"),
    }
}

#[test]
fn segment_size_overflow_at_base_plus_vaddr_rejects() {
    // Pick base + vaddr so the u64 addition wraps. base = u64::MAX,
    // vaddr = 1 trips `base.checked_add(seg.vaddr)`.
    let mut mem = fresh_memory();
    let parsed = ParsedPrx {
        name: "test_size_overflow".to_string(),
        module_id: module_id_from_name("test_size_overflow"),
        toc: 0,
        text: PrxSegment {
            vaddr: 1,
            filesz: 0,
            memsz: 0,
            data: vec![],
        },
        data: PrxSegment {
            vaddr: 2,
            filesz: 0,
            memsz: 0,
            data: vec![],
        },
        exports: vec![],
        relocations: vec![],
        module_start: None,
        module_stop: None,
    };
    let err = load_prx(&parsed, &mut mem, u64::MAX).unwrap_err();
    match err {
        PrxLoadError::SegmentSizeOverflow {
            segment,
            cause,
            vaddr,
            size,
        } => {
            assert_eq!(segment, "text");
            assert_eq!(cause, "base+vaddr");
            assert_eq!(vaddr, 1);
            assert_eq!(size, 0);
        }
        other => panic!("expected SegmentSizeOverflow, got {other:?}"),
    }
}

#[test]
fn segment_size_overflow_at_start_plus_size_rejects() {
    // base + vaddr fits, but start + memsz wraps. base = 0,
    // text.vaddr = u64::MAX - 1, text.memsz = 2 -> u64::MAX + 1
    // wraps. (memsz = 0 short-circuits to 0; bump to 2.)
    let mut mem = fresh_memory();
    let parsed = ParsedPrx {
        name: "test_end_overflow".to_string(),
        module_id: module_id_from_name("test_end_overflow"),
        toc: 0,
        text: PrxSegment {
            vaddr: u64::MAX - 1,
            filesz: 2,
            memsz: 2,
            data: vec![0u8; 2],
        },
        data: PrxSegment {
            vaddr: 0,
            filesz: 0,
            memsz: 0,
            data: vec![],
        },
        exports: vec![],
        relocations: vec![],
        module_start: None,
        module_stop: None,
    };
    let err = load_prx(&parsed, &mut mem, 0).unwrap_err();
    match err {
        PrxLoadError::SegmentSizeOverflow {
            segment,
            cause,
            vaddr,
            size,
        } => {
            assert_eq!(segment, "text");
            assert_eq!(cause, "start+size");
            assert_eq!(vaddr, u64::MAX - 1);
            assert_eq!(size, 2);
        }
        other => panic!("expected SegmentSizeOverflow, got {other:?}"),
    }
}
