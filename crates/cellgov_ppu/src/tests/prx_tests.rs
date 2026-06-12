//! PRX import-table parsing: synthetic-ELF round trips, bounds rejection, and error display.

use super::*;

#[test]
fn import_parse_error_display_renders_every_variant() {
    let cases: &[(ImportParseError, &[&str])] = &[
        (
            ImportParseError::NoImportsTable,
            &["PT_PRX_PARAM", "ppu_prx_library_info"],
        ),
        (
            ImportParseError::BadMagic(0xdead_beef),
            &["magic", "0xdeadbeef"],
        ),
        (
            ImportParseError::ParamHeaderTooSmall(8),
            &["header_size", "8"],
        ),
        (ImportParseError::OutOfBounds, &["past", "segment"]),
        (
            ImportParseError::BadImportsTableRange {
                start: 0x100,
                end: 0x80,
            },
            &["imports_table_end", "imports_table_start"],
        ),
        (
            ImportParseError::EntryTooSmall { declared: 0x10 },
            &["entry size byte", "0x10"],
        ),
        (
            ImportParseError::EntryPastImportsTable {
                entry_start: 0xd0,
                entry_size: 0x2c,
                imports_table_end: 0xe0,
            },
            &["0x000000d0", "0x000000e0", "44"],
        ),
        (
            ImportParseError::InvalidNamePtr { vaddr: 0x1234 },
            &["name_ptr", "0x00001234"],
        ),
        (
            ImportParseError::InvalidStubPtr {
                vaddr: 0x900,
                function_count: 5,
            },
            &["stub_ptr", "0x00000900", "5", "unmapped"],
        ),
        (
            ImportParseError::InvalidNidPtr {
                vaddr: 0x500,
                function_count: 3,
            },
            &["nid_ptr", "0x00000500", "3", "unmapped"],
        ),
    ];
    for (err, needles) in cases {
        let s = format!("{err}");
        assert!(!s.is_empty(), "empty Display for {err:?}");
        for needle in *needles {
            assert!(
                s.contains(needle),
                "Display of {err:?} missing {needle:?}: {s}"
            );
        }
    }
}

#[test]
#[ignore = "requires tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf; \
            run with CELLGOV_RETAIL_FIXTURES=1 cargo test -- --ignored"]
fn parse_retail_eboot_imports() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
    if !path.exists() {
        if std::env::var_os("CELLGOV_RETAIL_FIXTURES").is_some() {
            panic!(
                "CELLGOV_RETAIL_FIXTURES set but {} is absent",
                path.display()
            );
        }
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let modules = parse_imports(&data).unwrap();

    assert!(!modules.is_empty(), "should find imported modules");

    let total_funcs: usize = modules.iter().map(|m| m.functions.len()).sum();
    assert_eq!(modules.len(), 12);
    assert_eq!(total_funcs, 140);

    let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"cellSysutil"));
    assert!(names.contains(&"sysPrxForUser"));
    assert!(names.contains(&"cellGcmSys"));
}

/// Minimal ELF with PT_LOAD mapped 1:1 (vaddr == file offset) and
/// one import module of one function.
fn build_synthetic_prx_elf(nid: u32) -> Vec<u8> {
    const TOTAL_SIZE: usize = 320;
    const PARAM_OFF: usize = 176;
    const MOD_INFO_OFF: usize = 208;
    const MOD_INFO_SIZE: u8 = 0x2C;
    const NAME_OFF: usize = 252;
    const NID_TABLE_OFF: usize = 256;
    const STUB_TABLE_OFF: usize = 260;

    let mut data = vec![0u8; TOTAL_SIZE];
    // ELF header: magic, 64-bit, big-endian, phoff=64,
    // phentsize=56, phnum=2.
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&2u16.to_be_bytes());

    // PT_LOAD covering the whole file with vaddr == file offset.
    let ph0 = 64usize;
    data[ph0..ph0 + 4].copy_from_slice(&1u32.to_be_bytes());
    data[ph0 + 8..ph0 + 16].copy_from_slice(&0u64.to_be_bytes());
    data[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
    data[ph0 + 32..ph0 + 40].copy_from_slice(&(TOTAL_SIZE as u64).to_be_bytes());

    // PT_PRX_PARAM pointing at PARAM_OFF.
    let ph1 = 64 + 56;
    data[ph1..ph1 + 4].copy_from_slice(&PT_PRX_PARAM.to_be_bytes());
    data[ph1 + 8..ph1 + 16].copy_from_slice(&(PARAM_OFF as u64).to_be_bytes());

    // PrxParamHeader: header_size=0x40, magic, imports table.
    data[PARAM_OFF..PARAM_OFF + 4].copy_from_slice(&0x40u32.to_be_bytes());
    data[PARAM_OFF + 4..PARAM_OFF + 8].copy_from_slice(&PRX_PARAM_MAGIC.to_be_bytes());
    data[PARAM_OFF + 24..PARAM_OFF + 28].copy_from_slice(&(MOD_INFO_OFF as u32).to_be_bytes());
    data[PARAM_OFF + 28..PARAM_OFF + 32]
        .copy_from_slice(&(MOD_INFO_OFF as u32 + MOD_INFO_SIZE as u32).to_be_bytes());

    // PrxImportEntry: entry_size=0x2C, function_count=1,
    // name/nids/stubs ptrs.
    data[MOD_INFO_OFF] = MOD_INFO_SIZE;
    data[MOD_INFO_OFF + 6..MOD_INFO_OFF + 8].copy_from_slice(&1u16.to_be_bytes());
    data[MOD_INFO_OFF + 16..MOD_INFO_OFF + 20].copy_from_slice(&(NAME_OFF as u32).to_be_bytes());
    data[MOD_INFO_OFF + 20..MOD_INFO_OFF + 24]
        .copy_from_slice(&(NID_TABLE_OFF as u32).to_be_bytes());
    data[MOD_INFO_OFF + 24..MOD_INFO_OFF + 28]
        .copy_from_slice(&(STUB_TABLE_OFF as u32).to_be_bytes());

    data[NAME_OFF..NAME_OFF + 4].copy_from_slice(b"tst\0");
    data[NID_TABLE_OFF..NID_TABLE_OFF + 4].copy_from_slice(&nid.to_be_bytes());
    data[STUB_TABLE_OFF..STUB_TABLE_OFF + 4].copy_from_slice(&0u32.to_be_bytes());

    data
}

#[test]
fn parse_synthetic_elf_round_trips_one_module_one_function() {
    let nid = 0xDEAD_BEEFu32;
    let data = build_synthetic_prx_elf(nid);
    let modules = parse_imports(&data).expect("synthetic ELF must parse");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "tst");
    assert_eq!(modules[0].functions.len(), 1);
    assert_eq!(modules[0].functions[0].nid, nid);
    assert_eq!(modules[0].functions[0].stub_addr, 260);
}

#[test]
fn parse_rejects_param_header_too_small() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let param_off = 176;
    data[param_off..param_off + 4].copy_from_slice(&16u32.to_be_bytes());
    assert!(matches!(
        parse_imports(&data),
        Err(ImportParseError::ParamHeaderTooSmall(16))
    ));
}

#[test]
fn parse_rejects_unmapped_nid_table_when_function_count_nonzero() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    let unmapped_vaddr: u32 = 0xFFFF_0000;
    data[mod_info_off + 20..mod_info_off + 24].copy_from_slice(&unmapped_vaddr.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(
            err,
            ImportParseError::InvalidNidPtr {
                vaddr: 0xFFFF_0000,
                function_count: 1
            }
        ),
        "expected InvalidNidPtr, got {err:?}"
    );
}

#[test]
fn parse_rejects_entry_size_below_min() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    data[mod_info_off] = 0;
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::EntryTooSmall { declared: 0 }),
        "expected EntryTooSmall(0), got {err:?}"
    );
}

#[test]
fn parse_rejects_entry_size_below_canonical_min() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    data[mod_info_off] = 0x10;
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::EntryTooSmall { declared: 0x10 }),
        "expected EntryTooSmall(0x10), got {err:?}"
    );
}

#[test]
fn parse_rejects_imports_table_end_below_start() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let param_off = 176;
    let bad_start: u32 = 0x300;
    let bad_end: u32 = 0x200;
    data[param_off + 24..param_off + 28].copy_from_slice(&bad_start.to_be_bytes());
    data[param_off + 28..param_off + 32].copy_from_slice(&bad_end.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(
            err,
            ImportParseError::BadImportsTableRange {
                start: 0x300,
                end: 0x200
            }
        ),
        "expected BadImportsTableRange, got {err:?}"
    );
}

#[test]
fn parse_rejects_entry_extending_past_imports_table_end() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let param_off = 176;
    // Entry at 208 declares size 0x2C; truncate `imports_table_end`
    // to 208 + 16 so the entry's tail extends past it.
    let truncated_end: u32 = 208 + 16;
    data[param_off + 28..param_off + 32].copy_from_slice(&truncated_end.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert_eq!(
        err,
        ImportParseError::EntryPastImportsTable {
            entry_start: 208,
            entry_size: 0x2C,
            imports_table_end: 224,
        },
        "expected EntryPastImportsTable with the exact truncated end (224), got {err:?}"
    );
}

#[test]
fn parse_rejects_unmapped_name_ptr() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    let unmapped_vaddr: u32 = 0xFFFF_0000;
    data[mod_info_off + 16..mod_info_off + 20].copy_from_slice(&unmapped_vaddr.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::InvalidNamePtr { vaddr: 0xFFFF_0000 }),
        "expected InvalidNamePtr, got {err:?}"
    );
}

#[test]
fn parse_rejects_name_missing_nul_within_cap() {
    // Name region shorter than PRX_NAME_MAX_LEN, so segment end
    // (320 == TOTAL_SIZE) is the binding cap.
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let name_off = 252;
    for byte in &mut data[name_off..320] {
        *byte = b'A';
    }
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::InvalidNamePtr { .. }),
        "expected InvalidNamePtr from missing NUL, got {err:?}"
    );
}

#[test]
fn parse_rejects_unmapped_stub_ptr() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    let unmapped_vaddr: u32 = 0xFFFF_0000;
    data[mod_info_off + 24..mod_info_off + 28].copy_from_slice(&unmapped_vaddr.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(
            err,
            ImportParseError::InvalidStubPtr {
                vaddr: 0xFFFF_0000,
                function_count: 1
            }
        ),
        "expected InvalidStubPtr, got {err:?}"
    );
}

#[test]
fn parse_rejects_function_count_larger_than_nid_array_in_file() {
    // function_count = 100 but only one u32 of NID array bytes
    // lies within the segment.
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    let inflated: u16 = 100;
    data[mod_info_off + 6..mod_info_off + 8].copy_from_slice(&inflated.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::InvalidNidPtr { .. }),
        "expected InvalidNidPtr from function_count overflow, got {err:?}"
    );
}

#[test]
fn parse_synthetic_elf_function_count_zero_is_variable_only_module() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    let mod_info_off = 208;
    data[mod_info_off + 6..mod_info_off + 8].copy_from_slice(&0u16.to_be_bytes());
    let modules = parse_imports(&data).expect("variable-only module must parse");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "tst");
    assert!(modules[0].functions.is_empty());
}

#[test]
fn no_imports_table_returns_error() {
    let mut data = vec![0u8; 128];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&0u16.to_be_bytes());

    assert!(
        matches!(parse_imports(&data), Err(ImportParseError::NoImportsTable)),
        "expected NoImportsTable error"
    );
}

// File map (all hex):
//   0x000..0x040  ELF header (phoff=0x40, phentsize=56, phnum=1)
//   0x040..0x078  Phdr 0    (PT_LOAD; p_paddr = LIB_INFO_OFF)
//   0x0A0..0x0D4  library_info (52 bytes)
//   0x0D4..0x100  PrxImportEntry (0x2C bytes)
//   0x100..0x108  name "tst\0" + pad
//   0x108..0x10C  NID
//   0x10C..0x110  stub slot
fn build_library_info_prx_elf(nid: u32) -> Vec<u8> {
    const TOTAL_SIZE: usize = 320;
    const LIB_INFO_OFF: usize = 0xA0;
    const MOD_INFO_OFF: usize = 0xD4;
    const MOD_INFO_SIZE: u8 = 0x2C;
    const NAME_OFF: usize = 0x100;
    const NID_TABLE_OFF: usize = 0x108;
    const STUB_TABLE_OFF: usize = 0x10C;

    let mut data = vec![0u8; TOTAL_SIZE];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&1u16.to_be_bytes());

    // Phdr 0: PT_LOAD covering the whole file 1:1, with
    // p_paddr repurposed to point at LIB_INFO_OFF.
    let ph0 = 64usize;
    data[ph0..ph0 + 4].copy_from_slice(&1u32.to_be_bytes());
    // p_offset = 0
    data[ph0 + 8..ph0 + 16].copy_from_slice(&0u64.to_be_bytes());
    // p_vaddr = 0 (identity mapping)
    data[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
    // p_paddr = LIB_INFO_OFF (Sony repurpose)
    data[ph0 + 24..ph0 + 32].copy_from_slice(&(LIB_INFO_OFF as u64).to_be_bytes());
    // p_filesz = TOTAL_SIZE
    data[ph0 + 32..ph0 + 40].copy_from_slice(&(TOTAL_SIZE as u64).to_be_bytes());

    // library_info: imports_start at +44, imports_end at +48
    // (rest of the 52-byte struct stays zero -- attributes /
    // version / name / toc / exports are not consulted by
    // the import-path locator).
    data[LIB_INFO_OFF + 44..LIB_INFO_OFF + 48]
        .copy_from_slice(&(MOD_INFO_OFF as u32).to_be_bytes());
    data[LIB_INFO_OFF + 48..LIB_INFO_OFF + 52]
        .copy_from_slice(&(MOD_INFO_OFF as u32 + MOD_INFO_SIZE as u32).to_be_bytes());

    // PrxImportEntry: size=0x2C, num_func=1, three pointers.
    data[MOD_INFO_OFF] = MOD_INFO_SIZE;
    data[MOD_INFO_OFF + 6..MOD_INFO_OFF + 8].copy_from_slice(&1u16.to_be_bytes());
    data[MOD_INFO_OFF + 16..MOD_INFO_OFF + 20].copy_from_slice(&(NAME_OFF as u32).to_be_bytes());
    data[MOD_INFO_OFF + 20..MOD_INFO_OFF + 24]
        .copy_from_slice(&(NID_TABLE_OFF as u32).to_be_bytes());
    data[MOD_INFO_OFF + 24..MOD_INFO_OFF + 28]
        .copy_from_slice(&(STUB_TABLE_OFF as u32).to_be_bytes());

    data[NAME_OFF..NAME_OFF + 4].copy_from_slice(b"tst\0");
    data[NID_TABLE_OFF..NID_TABLE_OFF + 4].copy_from_slice(&nid.to_be_bytes());
    data[STUB_TABLE_OFF..STUB_TABLE_OFF + 4].copy_from_slice(&0u32.to_be_bytes());

    data
}

#[test]
fn parse_synthetic_library_info_path_round_trips_one_module() {
    let nid = 0xCAFE_BABEu32;
    let data = build_library_info_prx_elf(nid);
    let modules = parse_imports(&data).expect("library_info ELF must parse");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "tst");
    assert_eq!(modules[0].functions.len(), 1);
    assert_eq!(modules[0].functions[0].nid, nid);
    assert_eq!(modules[0].functions[0].stub_addr, 0x10C);
}

#[test]
fn parse_library_info_p_paddr_past_file_end_is_out_of_bounds() {
    let mut data = build_library_info_prx_elf(0xCAFE_BABE);
    let ph0 = 64usize;
    // p_paddr = TOTAL_SIZE - 10 (310): the 52-byte struct end
    // at +42 lands past the file's last byte.
    data[ph0 + 24..ph0 + 32].copy_from_slice(&310u64.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::OutOfBounds),
        "expected OutOfBounds for library_info past file end, got {err:?}"
    );
}

#[test]
fn parse_library_info_p_paddr_above_u32_max_is_out_of_bounds() {
    let mut data = build_library_info_prx_elf(0xCAFE_BABE);
    let ph0 = 64usize;
    data[ph0 + 24..ph0 + 32].copy_from_slice(&u64::MAX.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert!(
        matches!(err, ImportParseError::OutOfBounds),
        "expected OutOfBounds for huge p_paddr, got {err:?}"
    );
}

#[test]
fn parse_library_info_bad_imports_range_surfaces_bad_imports_table_range() {
    let mut data = build_library_info_prx_elf(0xCAFE_BABE);
    let lib_info_off = 0xA0;
    let bad_start: u32 = 0x300;
    let bad_end: u32 = 0x200;
    data[lib_info_off + 44..lib_info_off + 48].copy_from_slice(&bad_start.to_be_bytes());
    data[lib_info_off + 48..lib_info_off + 52].copy_from_slice(&bad_end.to_be_bytes());
    let err = parse_imports(&data).unwrap_err();
    assert_eq!(
        err,
        ImportParseError::BadImportsTableRange {
            start: 0x300,
            end: 0x200,
        }
    );
}

#[test]
fn parse_prefers_pt_prx_param_over_library_info_when_both_present() {
    let mut data = build_synthetic_prx_elf(0xDEAD_BEEF);
    // Synthetic fixture: TOTAL_SIZE=320, with bytes 0x108..0x140
    // unused. Place library_info there.
    let lib_info_off: usize = 0x108;
    let ph0 = 64usize;
    data[ph0 + 24..ph0 + 32].copy_from_slice(&(lib_info_off as u64).to_be_bytes());
    // library_info with bogus imports range (unmapped high
    // vaddrs). 52-byte struct fits in 0x108..0x13C.
    let bogus_start: u32 = 0xFFFF_0000;
    let bogus_end: u32 = 0xFFFF_0010;
    data[lib_info_off + 44..lib_info_off + 48].copy_from_slice(&bogus_start.to_be_bytes());
    data[lib_info_off + 48..lib_info_off + 52].copy_from_slice(&bogus_end.to_be_bytes());

    let modules = parse_imports(&data).expect("PT_PRX_PARAM precedence must hold");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name, "tst");
    assert_eq!(modules[0].functions.len(), 1);
    assert_eq!(modules[0].functions[0].nid, 0xDEAD_BEEF);
}
