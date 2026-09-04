//! What the applier does with a relocation that names a zero-sized
//! PT_LOAD placeholder, and with one that names no value segment.

use super::*;

use crate::prx_loader::graph::module_id_from_name;

const BASE: u64 = 0x1000_0000;
const TEXT_VADDR: u64 = 0x0;
const DATA_VADDR: u64 = 0x100;
const PLACEHOLDER_VADDR: u64 = 0x200;
const SEG_SIZE: u64 = 0x100;

/// Text at PT_LOAD 0, a zero-sized placeholder at PT_LOAD 1, data at
/// PT_LOAD 2. The placeholder declares a nonzero vaddr, so a
/// resolution against it lands at a visibly wrong address.
fn parsed_with_placeholder(relocs: Vec<PrxRelocation>) -> ParsedPrx {
    ParsedPrx {
        name: "placeholder_mod".to_string(),
        module_id: module_id_from_name("placeholder_mod"),
        toc: 0,
        text: PrxSegment {
            index: 0,
            vaddr: TEXT_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        data: PrxSegment {
            index: 2,
            vaddr: DATA_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        segment_vaddrs: vec![TEXT_VADDR, PLACEHOLDER_VADDR, DATA_VADDR],
        exports: vec![],
        relocations: relocs,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn a_relocation_resolving_against_an_empty_segment_is_refused() {
    let parsed = parsed_with_placeholder(vec![PrxRelocation {
        offset: 0x0,
        rtype: R_PPC64_ADDR32,
        sym: 0x0100,
        addend: 0x10,
    }]);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let err = load_prx(&parsed, &mut mem, BASE).expect_err("empty value segment");
    assert_eq!(
        err,
        PrxLoadError::RelocEmptyValueSegment {
            sym: 0x0100,
            seg: 1,
        }
    );
}

#[test]
fn a_relocation_targeting_an_empty_segment_reports_a_zero_sized_segment() {
    let parsed = parsed_with_placeholder(vec![PrxRelocation {
        offset: 0x0,
        rtype: R_PPC64_ADDR32,
        sym: 0x0001,
        addend: 0x10,
    }]);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let err = load_prx(&parsed, &mut mem, BASE).expect_err("empty target segment");
    assert_eq!(
        err,
        PrxLoadError::RelocOffsetOutOfSegment {
            rtype: R_PPC64_ADDR32,
            offset: 0x0,
            seg_size: 0,
        }
    );
}

#[test]
fn a_relocation_naming_no_value_segment_is_refused_at_load() {
    let parsed = parsed_with_placeholder(vec![PrxRelocation {
        offset: 0x0,
        rtype: R_PPC64_ADDR32,
        sym: 0xFF00,
        addend: 0x10,
    }]);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let err = load_prx(&parsed, &mut mem, BASE).expect_err("no value segment");
    assert_eq!(
        err,
        PrxLoadError::RelocWithoutValueSegment {
            rtype: R_PPC64_ADDR32,
            offset: 0x0,
        }
    );
}

#[test]
fn a_relocation_past_a_placeholder_resolves_against_the_segment_its_index_names() {
    let parsed = parsed_with_placeholder(vec![PrxRelocation {
        offset: 0x0,
        rtype: R_PPC64_ADDR32,
        sym: 0x0202,
        addend: 0x10,
    }]);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(loaded.relocs_applied, 1);

    let at = (BASE + DATA_VADDR) as usize;
    let word = u32::from_be_bytes([
        mem.as_bytes()[at],
        mem.as_bytes()[at + 1],
        mem.as_bytes()[at + 2],
        mem.as_bytes()[at + 3],
    ]);
    assert_eq!(word as u64, BASE + DATA_VADDR + 0x10);
}
