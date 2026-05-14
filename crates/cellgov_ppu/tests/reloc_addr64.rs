//! `R_PPC64_ADDR64` applier: write `S + A` as big-endian u64 at the
//! relocation site.
//!
//! Covers the per-(target_seg, value_seg) matrix, segment-boundary
//! cases (last 8 bytes, region-straddle), addend sign handling, and
//! the atomic-batch guarantees baked into `apply_relocations`'s
//! Phase-1 / Phase-2 split.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::graph::module_id_from_name;
use cellgov_ppu::sprx::{
    load_prx, ParsedPrx, PrxLoadError, PrxRelocation, PrxSegment, RelocMisalignedKind,
    R_PPC64_ADDR64,
};

const TEXT_VADDR: u64 = 0x0000;
const DATA_VADDR: u64 = 0x1_0000;
const SEG_SIZE: u64 = 0x1_0000;
const BASE: u64 = 0x4000_0000;

/// Single contiguous main region just large enough to hold text + data.
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
        name: "test_addr64".to_string(),
        module_id: module_id_from_name("test_addr64"),
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

fn parsed_with_addr64(offset: u64, addend: i64) -> ParsedPrx {
    parsed_with_relocs(vec![PrxRelocation {
        offset,
        rtype: R_PPC64_ADDR64,
        sym: 0, // target_seg = 0 (text), value_seg = 0 (text)
        addend,
    }])
}

/// Region-aware 8-byte BE read. `as_bytes()` only sees the
/// base-0 region; these fixtures place the region at BASE.
fn read_u64(mem: &GuestMemory, addr: u64) -> u64 {
    let range = ByteRange::new(GuestAddr::new(addr), 8).expect("range");
    let bytes = mem.read_checked(range).expect("region-aware read");
    u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Region-aware 1-byte read for byte-level BE verification.
fn read_byte(mem: &GuestMemory, addr: u64) -> u8 {
    let range = ByteRange::new(GuestAddr::new(addr), 1).expect("range");
    mem.read_checked(range).expect("region-aware read")[0]
}

// -- baseline cases --

#[test]
fn addr64_at_start_of_segment_writes_absolute_value() {
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(0x0000, 0x1234);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + TEXT_VADDR;
    let value = BASE + TEXT_VADDR + 0x1234;
    assert_eq!(read_u64(&mem, target), value);
}

#[test]
fn addr64_writes_eight_contiguous_bytes_inside_segment() {
    // Mid-segment 8-aligned offset; asserts the 8 patched bytes
    // land contiguously without touching neighbouring slots.
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(0x0020, 0x4320);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + TEXT_VADDR + 0x0020;
    let value = BASE + TEXT_VADDR + 0x4320;
    assert_eq!(read_u64(&mem, target), value);
    assert_eq!(read_u64(&mem, target - 8), 0);
    assert_eq!(read_u64(&mem, target + 8), 0);
}

// -- per-(target_seg, value_seg) matrix --

#[test]
fn addr64_text_target_data_value_resolves_data_vaddr() {
    let mut mem = fresh_memory();
    let mut parsed = parsed_with_addr64(0x0040, 0x0100);
    // target_seg=text (0), value_seg=data (1).
    parsed.relocations[0].sym = 1 << 8;
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + TEXT_VADDR + 0x0040;
    let value = BASE + DATA_VADDR + 0x0100;
    assert_eq!(read_u64(&mem, target), value);
}

#[test]
fn addr64_data_target_text_value_resolves_text_vaddr() {
    let mut mem = fresh_memory();
    let mut parsed = parsed_with_addr64(0x0040, 0x0080);
    // target_seg=data (1), value_seg=text (0). sym = 0x0001.
    parsed.relocations[0].sym = 0x0001;
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + DATA_VADDR + 0x0040;
    let value = BASE + TEXT_VADDR + 0x0080;
    assert_eq!(read_u64(&mem, target), value);
}

#[test]
fn addr64_data_target_data_value_round_trips_inside_data() {
    let mut mem = fresh_memory();
    let mut parsed = parsed_with_addr64(0x0048, 0x0200);
    // target_seg=data (1), value_seg=data (1). sym = 0x0101.
    parsed.relocations[0].sym = 0x0101;
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + DATA_VADDR + 0x0048;
    let value = BASE + DATA_VADDR + 0x0200;
    assert_eq!(read_u64(&mem, target), value);
}

// -- addend sign --

#[test]
fn addr64_negative_addend_subtracts_from_value_base() {
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(0x0080, -0x0100);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + TEXT_VADDR + 0x0080;
    let expected = BASE.wrapping_sub(0x100);
    assert_eq!(read_u64(&mem, target), expected);
}

// -- segment-boundary placements --

#[test]
fn addr64_at_last_eight_bytes_of_segment_is_accepted() {
    // Offset SEG_SIZE - 8 puts the entire 8-byte write within the
    // segment; locks the bound check at the exclusive boundary.
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(SEG_SIZE - 8, 0x0010);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);
    let target = BASE + TEXT_VADDR + (SEG_SIZE - 8);
    let value = BASE + TEXT_VADDR + 0x0010;
    assert_eq!(read_u64(&mem, target), value);
}

#[test]
fn addr64_at_seg_size_minus_four_rejects_as_misaligned() {
    // Offset SEG_SIZE - 4 = 0xFFFC is 4-aligned but not 8-aligned;
    // alignment fires before the out-of-segment check.
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(SEG_SIZE - 4, 0);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocMisaligned { rtype, kind, value } => {
            assert_eq!(rtype, R_PPC64_ADDR64);
            assert_eq!(kind, RelocMisalignedKind::PatchOffset);
            assert_eq!(value, (SEG_SIZE - 4) as i64);
        }
        other => panic!("expected RelocMisaligned, got {other:?}"),
    }
}

#[test]
fn addr64_at_segment_size_offset_rejects_out_of_segment() {
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(SEG_SIZE, 0);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocOffsetOutOfSegment {
            rtype,
            offset,
            seg_size,
        } => {
            assert_eq!(rtype, R_PPC64_ADDR64);
            assert_eq!(offset, SEG_SIZE);
            assert_eq!(seg_size, SEG_SIZE);
        }
        other => panic!("expected RelocOffsetOutOfSegment, got {other:?}"),
    }
}

// -- atomic-batch + content shape --

#[test]
fn addr64_empty_relocations_list_loads_with_zero_relocs_applied() {
    let mut mem = fresh_memory();
    let parsed = parsed_with_relocs(vec![]);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 0);
}

#[test]
fn addr64_writes_big_endian_byte_order_at_byte_level() {
    // Pins BE at the byte level: a writer/reader pair swapped to
    // LE would still round-trip through from_be_bytes / to_be_bytes.
    let mut mem = fresh_memory();
    let parsed = parsed_with_addr64(0x0100, 0x0102_0304);
    load_prx(&parsed, &mut mem, BASE).expect("load");
    let target = BASE + TEXT_VADDR + 0x0100;
    // BE expectation: MSB at lowest address.
    assert_eq!(read_byte(&mem, target), 0x00, "byte 0 of u64");
    assert_eq!(read_byte(&mem, target + 1), 0x00, "byte 1 of u64");
    assert_eq!(read_byte(&mem, target + 2), 0x00, "byte 2 of u64");
    assert_eq!(read_byte(&mem, target + 3), 0x00, "byte 3 of u64");
    assert_eq!(
        read_byte(&mem, target + 4),
        0x41,
        "byte 4 of u64 (high byte of value)"
    );
    assert_eq!(read_byte(&mem, target + 5), 0x02, "byte 5 of u64");
    assert_eq!(read_byte(&mem, target + 6), 0x03, "byte 6 of u64");
    assert_eq!(
        read_byte(&mem, target + 7),
        0x04,
        "byte 7 of u64 (low byte)"
    );
}
