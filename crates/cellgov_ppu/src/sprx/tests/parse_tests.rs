//! PRX parsing: segments, exports, module entry points, and relocation records.

use super::*;
use crate::sprx::{R_PPC64_ADDR16_HA, R_PPC64_ADDR16_HI, R_PPC64_ADDR16_LO, R_PPC64_ADDR32};

use crate::sprx::test_fixtures::make_test_prx;

#[test]
fn parse_test_prx_basic() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    assert_eq!(prx.name, "testmod");
    assert_eq!(prx.toc, 0x200);
}

#[test]
fn parse_test_prx_segments() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    assert_eq!(prx.text.vaddr, 0);
    assert_eq!(prx.text.filesz, 0x100);
    assert_eq!(prx.data.vaddr, 0x100);
    assert_eq!(prx.data.filesz, 0x200);
}

#[test]
fn parse_test_prx_exports() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    assert_eq!(prx.exports.len(), 1);
    assert_eq!(prx.exports[0].name, "testlib");
    assert_eq!(prx.exports[0].functions.len(), 3);
    assert_eq!(prx.exports[0].functions[0].nid, 0xAAAAAAAA);
    assert_eq!(prx.exports[0].functions[1].nid, 0xBBBBBBBB);
    assert_eq!(prx.exports[0].functions[2].nid, 0xCCCCCCCC);
    assert_eq!(prx.exports[0].functions[0].vaddr, 0x40);
}

#[test]
fn parse_test_prx_module_start_stop() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let ms = prx.module_start.expect("module_start should be present");
    assert_eq!(ms.opd_vaddr, 0x1F0);
    assert_eq!(ms.code, 0x10);
    assert_eq!(ms.toc, 0x200);

    let mstop = prx.module_stop.expect("module_stop should be present");
    assert_eq!(mstop.opd_vaddr, 0x1F8);
    assert_eq!(mstop.code, 0x20);
    assert_eq!(mstop.toc, 0x200);
}

#[test]
fn parse_test_prx_relocations() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    assert_eq!(prx.relocations.len(), 3);
    assert_eq!(prx.relocations[0].offset, 0x50);
    assert_eq!(prx.relocations[0].rtype, R_PPC64_ADDR32);
    assert_eq!(prx.relocations[0].sym, 0);
    assert_eq!(prx.relocations[0].addend, 0x80);
    assert_eq!(prx.relocations[1].offset, 0x54);
    assert_eq!(prx.relocations[1].rtype, R_PPC64_ADDR16_HA);
    assert_eq!(prx.relocations[1].addend, 0x200);
    assert_eq!(prx.relocations[2].offset, 0xF0);
    assert_eq!(prx.relocations[2].rtype, R_PPC64_ADDR32);
    assert_eq!(prx.relocations[2].sym, 0x0001);
    assert_eq!(prx.relocations[2].addend, 0x10);
}

#[test]
fn reject_non_prx_elf() {
    let mut data = vec![0u8; 128];
    data[0..4].copy_from_slice(&ELF_MAGIC);
    data[4] = 2;
    data[5] = 2;
    data[16..18].copy_from_slice(&0x0002u16.to_be_bytes()); // ET_EXEC

    assert!(matches!(parse_prx(&data), Err(PrxParseError::NotPrx(2))));
}

#[test]
fn reject_too_small() {
    assert!(matches!(parse_prx(&[0; 10]), Err(PrxParseError::TooSmall)));
}

#[test]
fn reject_bad_magic() {
    let data = vec![0u8; 128];
    assert!(matches!(parse_prx(&data), Err(PrxParseError::BadMagic)));
}

#[test]
fn parse_real_liblv2() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_flash_decrypted/sys/external/liblv2.prx");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let prx = parse_prx(&data).unwrap();

    assert_eq!(prx.name, "liblv2");
    assert_eq!(prx.toc, 0x1c620);

    let spy = prx
        .exports
        .iter()
        .find(|e| e.name == "sysPrxForUser")
        .expect("liblv2 should export sysPrxForUser");
    assert_eq!(spy.functions.len(), 157);

    let ms = prx.module_start.expect("liblv2 should have module_start");
    assert_eq!(ms.code, 0x0);
    assert_eq!(ms.toc, 0x1c620);

    assert!(
        prx.relocations.len() > 1000,
        "expected >1000 relocs, got {}",
        prx.relocations.len()
    );

    for r in &prx.relocations {
        assert!(
            matches!(
                r.rtype,
                R_PPC64_ADDR32 | R_PPC64_ADDR16_LO | R_PPC64_ADDR16_HI | R_PPC64_ADDR16_HA
            ),
            "unexpected reloc type {} at offset 0x{:x}",
            r.rtype,
            r.offset
        );
    }
}

#[test]
fn parse_rejects_phentsize_below_minimum() {
    let mut data = make_test_prx();
    data[54..56].copy_from_slice(&8u16.to_be_bytes());
    assert!(matches!(parse_prx(&data), Err(PrxParseError::OutOfBounds)));
}

#[test]
fn read_cstring_unmapped_pointer_produces_diagnostic_string() {
    let mut data = make_test_prx();
    // exp0 lives at file 0x22; exp1 is 28 bytes after.
    let exp1 = 0x224 + 28;
    let unmapped: u32 = 0xDEAD_0000;
    data[exp1 + 16..exp1 + 20].copy_from_slice(&unmapped.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    assert_eq!(prx.exports.len(), 1);
    assert!(
        prx.exports[0].name.starts_with("<unmapped:0x"),
        "expected diagnostic name for unmapped lib_name_ptr, got {:?}",
        prx.exports[0].name
    );
}
