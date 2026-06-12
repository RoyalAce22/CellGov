//! PRX segment loading into guest memory with export and entry-point relocation.

use super::*;
use crate::sprx::parse_prx;

use crate::sprx::test_fixtures::make_test_prx;

#[test]
fn load_test_prx_segments() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let base: u64 = 0x1000_0000;
    let mem_size = 0x2000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(mem_size);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    assert_eq!(loaded.name, "testmod");
    assert_eq!(loaded.base, base);
    assert_eq!(loaded.toc, base + 0x200);
    assert_eq!(loaded.text_start, base);
    assert_eq!(loaded.text_end, base + 0x100);
    assert_eq!(loaded.data_start, base + 0x100);
    assert_eq!(loaded.data_end, base + 0x300);
}

#[test]
fn load_test_prx_exports_relocated() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    assert_eq!(loaded.exports.len(), 3);
    assert_eq!(loaded.exports[&0xAAAAAAAA], base + 0x40);
    assert_eq!(loaded.exports[&0xBBBBBBBB], base + 0x50);
    assert_eq!(loaded.exports[&0xCCCCCCCC], base + 0x60);
}

#[test]
fn load_test_prx_module_start_relocated() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    let ms = loaded.module_start.expect("module_start");
    assert_eq!(ms.code, base + 0x10);
    assert_eq!(ms.toc, base + 0x200);

    let mstop = loaded.module_stop.expect("module_stop");
    assert_eq!(mstop.code, base + 0x20);
    assert_eq!(mstop.toc, base + 0x200);
}

#[test]
fn load_test_prx_relocations_applied() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    assert_eq!(loaded.relocs_applied, 3);

    // ADDR32 text->text: target base+0x50, value base+0x80.
    let addr = (base + 0x50) as usize;
    let val = u32::from_be_bytes([
        mem.as_bytes()[addr],
        mem.as_bytes()[addr + 1],
        mem.as_bytes()[addr + 2],
        mem.as_bytes()[addr + 3],
    ]);
    assert_eq!(val, 0x1000_0080, "ADDR32 text->text mismatch");

    // ADDR16_HA: value 0x1000_0200, HA = (value + 0x8000) >> 16.
    let addr2 = (base + 0x54) as usize;
    let val2 = u16::from_be_bytes([mem.as_bytes()[addr2], mem.as_bytes()[addr2 + 1]]);
    assert_eq!(val2, 0x1000, "ADDR16_HA mismatch");

    // ADDR32 data->text patches module_start OPD code field.
    let addr3 = (base + 0x1F0) as usize;
    let val3 = u32::from_be_bytes([
        mem.as_bytes()[addr3],
        mem.as_bytes()[addr3 + 1],
        mem.as_bytes()[addr3 + 2],
        mem.as_bytes()[addr3 + 3],
    ]);
    assert_eq!(val3, 0x1000_0010, "ADDR32 data->text (OPD) mismatch");
}

#[test]
fn load_test_prx_addr16_lo_and_hi() {
    let mut data = make_test_prx();

    let ph2 = 64 + 112;
    data[ph2 + 32..ph2 + 40].copy_from_slice(&48u64.to_be_bytes());

    let rel0 = 0x3F0;
    data[rel0..rel0 + 8].copy_from_slice(&0x58u64.to_be_bytes());
    let r_info0: u64 = R_PPC64_ADDR16_LO as u64;
    data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
    data[rel0 + 16..rel0 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

    let rel1 = rel0 + 24;
    data[rel1..rel1 + 8].copy_from_slice(&0x5Au64.to_be_bytes());
    let r_info1: u64 = R_PPC64_ADDR16_HI as u64;
    data[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
    data[rel1 + 16..rel1 + 24].copy_from_slice(&0x12345678i64.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();
    assert_eq!(loaded.relocs_applied, 2);

    // value = 0x1000_0000 + 0x12345678 = 0x2234_5678.
    let addr_lo = (base + 0x58) as usize;
    let lo = u16::from_be_bytes([mem.as_bytes()[addr_lo], mem.as_bytes()[addr_lo + 1]]);
    assert_eq!(lo, 0x5678, "ADDR16_LO mismatch");

    let addr_hi = (base + 0x5A) as usize;
    let hi = u16::from_be_bytes([mem.as_bytes()[addr_hi], mem.as_bytes()[addr_hi + 1]]);
    assert_eq!(hi, 0x2234, "ADDR16_HI mismatch");
}

#[test]
fn load_prx_rejects_out_of_range() {
    let data = make_test_prx();
    let prx = parse_prx(&data).unwrap();

    let mut mem = cellgov_mem::GuestMemory::new(0x100);
    let result = load_prx(&prx, &mut mem, 0x1000_0000);
    assert!(matches!(
        result,
        Err(PrxLoadError::SegmentOutOfRange { .. })
    ));
}

#[test]
fn load_prx_rejects_reloc_with_out_of_range_segment() {
    // value_seg = 0x02 against a 2-entry [text, data] table.
    let mut data = make_test_prx();
    let rel0 = 0x3F0;
    let r_info: u64 = (0x0200u64 << 32) | R_PPC64_ADDR32 as u64;
    data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let result = load_prx(&prx, &mut mem, 0x1000_0000);
    assert!(matches!(
        result,
        Err(PrxLoadError::RelocSegmentOutOfRange { seg: 2, .. })
    ));
}

#[test]
fn load_module_start_not_double_added_when_text_vaddr_nonzero() {
    let mut data = make_test_prx();
    let ph0 = 64;
    let new_text_vaddr: u64 = 0x1000;
    data[ph0 + 16..ph0 + 24].copy_from_slice(&new_text_vaddr.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    assert_eq!(prx.text.vaddr, 0x1000);
    assert_eq!(
        prx.module_start.expect("module_start").code,
        0x10,
        "OPD code is absolute PRX vaddr, not text-relative",
    );

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    let ms = loaded.module_start.expect("module_start");
    assert_eq!(
        ms.code,
        base + 0x10,
        "ms.code = base + opd.code, not base + text.vaddr + opd.code",
    );
}

#[test]
fn load_uses_per_opd_toc_not_module_info_toc() {
    let mut data = make_test_prx();
    let opd_base = 0x2E0;
    let alt_toc: u32 = 0x300;
    data[opd_base + 4..opd_base + 8].copy_from_slice(&alt_toc.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    assert_eq!(prx.toc, 0x200);
    assert_eq!(prx.module_start.expect("module_start").toc, alt_toc);

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    let ms = loaded.module_start.expect("module_start");
    assert_eq!(ms.toc, base + alt_toc as u64);
    // module_stop's OPD still carries 0x200; divergence proves per-OPD.
    let mstop = loaded.module_stop.expect("module_stop");
    assert_eq!(mstop.toc, base + 0x200);
}

#[test]
fn applier_supported_types_match_apply_relocations() {
    // Feed each type in APPLIER_SUPPORTED_TYPES through
    // apply_relocations and reject UnsupportedReloc. Other errors
    // (overflow, write failure) are fine -- absence of
    // UnsupportedReloc is the invariant.
    let text = PrxSegment {
        vaddr: 0,
        filesz: 0x100,
        memsz: 0x100,
        data: vec![0u8; 0x100],
    };
    let data = PrxSegment {
        vaddr: 0,
        filesz: 0x100,
        memsz: 0x100,
        data: vec![0u8; 0x100],
    };
    for &rtype in APPLIER_SUPPORTED_TYPES {
        let relocs = vec![PrxRelocation {
            offset: 0,
            rtype,
            sym: 0,
            addend: 0,
        }];
        let mut staging = cellgov_mem::StagingMemory::new();
        let result = apply_relocations(&mut staging, 0, &text, &data, &relocs);
        staging.clear();
        match result {
            Ok(_) => {}
            Err(PrxLoadError::UnsupportedReloc(t)) => panic!(
                "type {t} listed in APPLIER_SUPPORTED_TYPES but apply_relocations has no match arm"
            ),
            Err(_) => {}
        }
    }
}

#[test]
fn is_applier_supported_matches_const_list() {
    for &t in APPLIER_SUPPORTED_TYPES {
        assert!(
            is_applier_supported(t),
            "type {t} missing from is_applier_supported"
        );
    }
    assert!(!is_applier_supported(99), "type 99 is not covered");
    assert!(!is_applier_supported(0), "type 0 (NONE) is not covered");
}

/// Bidirectional check: every reloc-type integer in a swept
/// range that is NOT in APPLIER_SUPPORTED_TYPES must trigger
/// UnsupportedReloc, proving `apply_relocations` does not silently
/// accept any type missing from the const list. The complementary
/// direction (`applier_supported_types_match_apply_relocations`
/// above) proves every type in the const list IS handled.
/// Together: const list and applier match-arm set are equal.
#[test]
fn unsupported_reloc_types_rejected_outside_const_list() {
    use std::collections::BTreeSet;
    let supported: BTreeSet<u32> = APPLIER_SUPPORTED_TYPES.iter().copied().collect();
    let text = PrxSegment {
        vaddr: 0,
        filesz: 0x100,
        memsz: 0x100,
        data: vec![0u8; 0x100],
    };
    let data = PrxSegment {
        vaddr: 0,
        filesz: 0x100,
        memsz: 0x100,
        data: vec![0u8; 0x100],
    };
    // 0..=120 covers the R_PPC64 reloc-type integer range
    // including R_PPC64_REL24 (10) and the ADDR16_*_DS family
    // (56-58). Beyond this every value is reserved or vendor.
    for rtype in 0u32..=120 {
        if supported.contains(&rtype) {
            continue;
        }
        let relocs = vec![PrxRelocation {
            offset: 0,
            rtype,
            sym: 0,
            addend: 0,
        }];
        let mut staging = cellgov_mem::StagingMemory::new();
        let result = apply_relocations(&mut staging, 0, &text, &data, &relocs);
        staging.clear();
        match result {
            Err(PrxLoadError::UnsupportedReloc(t)) => assert_eq!(
                t, rtype,
                "UnsupportedReloc reported wrong type ({t}) for input {rtype}",
            ),
            Ok(_) => panic!(
                "type {rtype} not in APPLIER_SUPPORTED_TYPES but \
                 apply_relocations accepted it -- missing const-list entry",
            ),
            Err(other) => panic!(
                "type {rtype} produced non-UnsupportedReloc error {other:?}; \
                 either add to APPLIER_SUPPORTED_TYPES or extend the test fixture",
            ),
        }
    }
}

#[test]
fn load_prx_rejects_unsupported_reloc() {
    let mut data = make_test_prx();

    let rel0 = 0x3F0;
    let r_info: u64 = 99;
    data[rel0 + 8..rel0 + 16].copy_from_slice(&r_info.to_be_bytes());

    let prx = parse_prx(&data).unwrap();
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let result = load_prx(&prx, &mut mem, 0x1000_0000);
    assert!(matches!(result, Err(PrxLoadError::UnsupportedReloc(99))));
}

#[test]
fn load_real_liblv2() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_flash_decrypted/sys/external/liblv2.prx");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let prx = parse_prx(&data).unwrap();

    let base: u64 = 0x1000_0000;
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let loaded = load_prx(&prx, &mut mem, base).unwrap();

    assert_eq!(loaded.name, "liblv2");
    assert_eq!(loaded.base, base);
    assert_eq!(loaded.toc, base + 0x1c620);
    assert!(loaded.relocs_applied > 1000);

    let ms = loaded.module_start.expect("module_start");
    assert_eq!(ms.code, base, "module_start code should be at base");
    assert_eq!(ms.toc, base + 0x1c620, "module_start TOC");

    let text_start = base as usize;
    let first_insn = u32::from_be_bytes([
        mem.as_bytes()[text_start],
        mem.as_bytes()[text_start + 1],
        mem.as_bytes()[text_start + 2],
        mem.as_bytes()[text_start + 3],
    ]);
    let opcode = first_insn >> 26;
    assert!(
        opcode > 0 && opcode < 64,
        "first instruction should be valid PPC64, got 0x{:08x}",
        first_insn
    );

    assert!(
        loaded
            .exports
            .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::INITIALIZE_TLS),
        "should export sys_initialize_tls"
    );
    assert!(
        loaded
            .exports
            .contains_key(&cellgov_ps3_abi::nid::sys_prx_for_user::MALLOC),
        "should export _sys_malloc"
    );
}
