//! Pointer slots stored as a bare addend parse to the same addresses as
//! slots that store the resolved sum. Older firmware modules ship the
//! addend form.

use crate::sprx::parse_prx;
use crate::sprx::test_fixtures::{make_test_prx, make_test_prx_graph_node};
use crate::sprx::R_PPC64_ADDR32;
use cellgov_ps3_abi::format::elf::ELF64_RELA_SIZE;

// Fixture geometry from `make_test_prx`.
const DATA_VADDR: u32 = 0x100;
const DATA_FILE_OFF: usize = 0x1F0;
const RELOC_FILE_OFF: usize = 0x3F0;
const RELOC_OFFSET_FIELD: usize = 64 + 112 + 8;
const RELOC_FILESZ_FIELD: usize = 64 + 112 + 32;

/// Rewrite each slot in `slots` to a bare addend and append its relocation.
///
/// Slot and value both sit in the fixture's data segment, so each appended
/// entry is `R_PPC64_ADDR32` with target and value segment 1.
fn store_pointers_as_addends(buf: &mut Vec<u8>, slots: &[usize]) {
    let mut count = u64::from_be_bytes(
        buf[RELOC_FILESZ_FIELD..RELOC_FILESZ_FIELD + 8]
            .try_into()
            .expect("8-byte filesz"),
    ) as usize
        / ELF64_RELA_SIZE;

    for &slot in slots {
        let resolved = u32::from_be_bytes(buf[slot..slot + 4].try_into().expect("4-byte slot"));
        assert!(
            resolved >= DATA_VADDR,
            "slot 0x{slot:x} holds 0x{resolved:08x}, which is not a data-segment pointer",
        );
        let addend = resolved - DATA_VADDR;
        buf[slot..slot + 4].copy_from_slice(&addend.to_be_bytes());

        let entry = RELOC_FILE_OFF + count * ELF64_RELA_SIZE;
        if buf.len() < entry + ELF64_RELA_SIZE {
            buf.resize(entry + ELF64_RELA_SIZE, 0);
        }
        let seg_off = (slot - DATA_FILE_OFF) as u64;
        let r_info: u64 = (0x0101u64 << 32) | u64::from(R_PPC64_ADDR32);
        buf[entry..entry + 8].copy_from_slice(&seg_off.to_be_bytes());
        buf[entry + 8..entry + 16].copy_from_slice(&r_info.to_be_bytes());
        buf[entry + 16..entry + 24].copy_from_slice(&i64::from(addend).to_be_bytes());
        count += 1;
    }

    let filesz = (count * ELF64_RELA_SIZE) as u64;
    buf[RELOC_FILESZ_FIELD..RELOC_FILESZ_FIELD + 8].copy_from_slice(&filesz.to_be_bytes());
}

/// Module-info TOC and export ranges, the system entry's stub-table
/// pointer, the two OPD addresses it holds, and each OPD's TOC word.
const EXPORT_POINTER_SLOTS: [usize; 8] = [0x210, 0x214, 0x218, 0x23C, 0x2A0, 0x2A4, 0x2E4, 0x2EC];

/// Module-info import range plus the import entry's name, NID-table and
/// stub-table pointers.
const IMPORT_POINTER_SLOTS: [usize; 5] = [0x21C, 0x220, 0x310, 0x314, 0x318];

#[test]
fn addend_pointers_produce_the_same_module_info_and_entry_points() {
    let prebaked = parse_prx(&make_test_prx()).expect("prebaked parse");

    let mut buf = make_test_prx();
    store_pointers_as_addends(&mut buf, &EXPORT_POINTER_SLOTS);
    let addend_form = parse_prx(&buf).expect("addend-form parse");

    assert_eq!(addend_form.toc, prebaked.toc);
    assert_eq!(
        addend_form
            .module_start
            .map(|o| (o.opd_vaddr, o.code, o.toc)),
        prebaked.module_start.map(|o| (o.opd_vaddr, o.code, o.toc)),
    );
    assert_eq!(
        addend_form
            .module_stop
            .map(|o| (o.opd_vaddr, o.code, o.toc)),
        prebaked.module_stop.map(|o| (o.opd_vaddr, o.code, o.toc)),
    );
}

#[test]
fn reading_an_addend_slot_raw_lands_in_the_wrong_segment() {
    let mut buf = make_test_prx();
    store_pointers_as_addends(&mut buf, &EXPORT_POINTER_SLOTS);

    let raw_opd_vaddr = u32::from_be_bytes(buf[0x2A0..0x2A4].try_into().expect("4-byte slot"));
    assert!(
        raw_opd_vaddr < DATA_VADDR,
        "the fixture must store an addend the raw read resolves into text",
    );
    let opd = parse_prx(&buf)
        .expect("addend-form parse")
        .module_start
        .expect("module_start");
    assert!(
        opd.opd_vaddr >= DATA_VADDR,
        "module_start OPD 0x{:08x} resolved into the text segment",
        opd.opd_vaddr,
    );
}

#[test]
fn addend_pointers_produce_the_same_import_table() {
    let node = || make_test_prx_graph_node("modaaaa", "libaaaa", Some("implib"));
    let prebaked = crate::prx::parse_imports(&node()).expect("prebaked imports");

    let mut buf = node();
    store_pointers_as_addends(&mut buf, &IMPORT_POINTER_SLOTS);
    let addend_form = crate::prx::parse_imports(&buf).expect("addend-form imports");

    assert_eq!(addend_form.len(), 1, "fixture declares one imported module");
    assert_eq!(addend_form[0].name, prebaked[0].name);
    assert_eq!(
        addend_form[0]
            .functions
            .iter()
            .map(|f| (f.nid, f.stub_addr))
            .collect::<Vec<_>>(),
        prebaked[0]
            .functions
            .iter()
            .map(|f| (f.nid, f.stub_addr))
            .collect::<Vec<_>>(),
    );
}

#[test]
fn a_relocation_segment_running_past_the_file_is_refused_not_read_raw() {
    let mut buf = make_test_prx_graph_node("modaaaa", "libaaaa", Some("implib"));
    crate::prx::parse_imports(&buf).expect("intact fixture parses");

    let past_end = buf.len() as u64;
    buf[RELOC_FILESZ_FIELD..RELOC_FILESZ_FIELD + 8].copy_from_slice(&past_end.to_be_bytes());

    let err = crate::prx::parse_imports(&buf).expect_err("declared reloc segment escapes the file");
    assert!(
        matches!(err, crate::prx::ImportParseError::OutOfBounds),
        "expected the escape to be named, got {err:?}",
    );
}

#[test]
fn a_relocation_segment_whose_file_offset_wraps_is_refused_not_panicked_on() {
    let mut buf = make_test_prx_graph_node("modaaaa", "libaaaa", Some("implib"));
    buf[RELOC_OFFSET_FIELD..RELOC_OFFSET_FIELD + 8].copy_from_slice(&(u64::MAX - 8).to_be_bytes());
    buf[RELOC_FILESZ_FIELD..RELOC_FILESZ_FIELD + 8].copy_from_slice(&0x100u64.to_be_bytes());

    let err = crate::prx::parse_imports(&buf).expect_err("wrapping reloc file offset");
    assert!(
        matches!(err, crate::prx::ImportParseError::OutOfBounds),
        "expected the wrap to be named, got {err:?}",
    );
}
