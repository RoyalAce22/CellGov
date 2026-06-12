//! Firmware-set loading: module_start order, relocation checks, and atomic import patching.

use super::*;
use crate::sprx::LoadedOpd;

fn stub_loaded(id: PrxModuleId, has_start: bool) -> LoadedPrx {
    LoadedPrx {
        name: format!("m{}", id.0),
        module_id: id,
        base: 0,
        toc: 0,
        text_start: 0,
        text_end: 0,
        data_start: 0,
        data_end: 0,
        exports: BTreeMap::new(),
        module_start: has_start.then_some(LoadedOpd {
            code: 0x1000 + u64::from(id.0),
            toc: 0,
        }),
        module_stop: None,
        relocs_applied: 0,
    }
}

struct Recorder {
    calls: Vec<PrxModuleId>,
}

impl ModuleStartRunner for Recorder {
    fn run_module_start(
        &mut self,
        module: &LoadedPrx,
        _opd: LoadedOpd,
    ) -> Result<(), ModuleStartRunError> {
        self.calls.push(module.module_id);
        Ok(())
    }
}

fn image_with_order(order: Vec<PrxModuleId>, with_start: &[PrxModuleId]) -> FirmwareImage {
    let loaded: BTreeMap<_, _> = order
        .iter()
        .map(|id| (*id, stub_loaded(*id, with_start.contains(id))))
        .collect();
    FirmwareImage {
        loaded,
        export_table: FirmwareExportTable::default(),
        topological_order: order,
        imports_by_id: BTreeMap::new(),
    }
}

#[test]
fn start_modules_iterates_topological_order_field() {
    let order = vec![PrxModuleId(1), PrxModuleId(2), PrxModuleId(3)];
    let image = image_with_order(order.clone(), &order);
    let mut rec = Recorder { calls: Vec::new() };
    start_modules(&image, &mut rec).expect("start");
    assert_eq!(rec.calls, order);
}

#[test]
fn start_modules_skips_modules_without_module_start() {
    let order = vec![PrxModuleId(1), PrxModuleId(2), PrxModuleId(3)];
    let image = image_with_order(order, &[PrxModuleId(2)]);
    let mut rec = Recorder { calls: Vec::new() };
    start_modules(&image, &mut rec).expect("start");
    assert_eq!(rec.calls, vec![PrxModuleId(2)]);
}

struct FailingRunner;
impl ModuleStartRunner for FailingRunner {
    fn run_module_start(
        &mut self,
        module: &LoadedPrx,
        _opd: LoadedOpd,
    ) -> Result<(), ModuleStartRunError> {
        Err(ModuleStartRunError::RunnerReported {
            reason: format!("synthetic: {}", module.name),
        })
    }
}

fn stub_parsed(id: PrxModuleId, relocs: Vec<crate::sprx::PrxRelocation>) -> crate::sprx::ParsedPrx {
    crate::sprx::ParsedPrx {
        name: format!("synth-{}", id.0),
        module_id: id,
        toc: 0,
        text: crate::sprx::PrxSegment {
            vaddr: 0,
            filesz: 0,
            memsz: 0,
            data: Vec::new(),
        },
        data: crate::sprx::PrxSegment {
            vaddr: 0,
            filesz: 0,
            memsz: 0,
            data: Vec::new(),
        },
        exports: Vec::new(),
        relocations: relocs,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn check_loadable_flags_relocation_into_third_segment() {
    let parsed = stub_parsed(
        PrxModuleId(7),
        vec![crate::sprx::PrxRelocation {
            offset: 0,
            rtype: 1,
            sym: 0x0203, // target_seg=3, value_seg=2
            addend: 0,
        }],
    );
    let err = check_relocations_within_text_data(&parsed).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::MultiSegmentRelocations {
            module: PrxModuleId(7),
            segment_idx: 3,
        }
    );
}

#[test]
fn check_loadable_flags_value_segment_alone_when_target_is_text() {
    // sym 0x0200 = target_seg=0 (text), value_seg=2 (out of range).
    let parsed = stub_parsed(
        PrxModuleId(9),
        vec![crate::sprx::PrxRelocation {
            offset: 0,
            rtype: 1,
            sym: 0x0200, // target_seg=0, value_seg=2
            addend: 0,
        }],
    );
    let err = check_relocations_within_text_data(&parsed).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::MultiSegmentRelocations {
            module: PrxModuleId(9),
            segment_idx: 2,
        }
    );
}

#[test]
fn check_loadable_accepts_text_and_data_only_relocations() {
    let parsed = stub_parsed(
        PrxModuleId(8),
        vec![
            crate::sprx::PrxRelocation {
                offset: 0,
                rtype: 1,
                sym: 0x0000,
                addend: 0,
            },
            crate::sprx::PrxRelocation {
                offset: 0,
                rtype: 1,
                sym: 0x0101,
                addend: 0,
            },
        ],
    );
    assert!(check_relocations_within_text_data(&parsed).is_ok());
}

#[test]
fn start_modules_propagates_runner_error_with_reason_preserved() {
    let id = PrxModuleId(7);
    let image = image_with_order(vec![id], &[id]);
    let err = start_modules(&image, &mut FailingRunner).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::ModuleStartFailed {
            module: id,
            reason: "synthetic: m7".to_string(),
        }
    );
}

#[test]
fn synthetic_game_elf_id_equals_module_id_from_name_of_empty_string() {
    assert_eq!(SYNTHETIC_GAME_ELF_ID, graph::module_id_from_name(""));
}

#[test]
fn module_id_from_name_is_stable_for_liblv2() {
    // sync_state_hash transitively depends on the FNV-1a-32
    // mapping for "liblv2" being byte-stable across runs and
    // hosts; drift here is a determinism regression.
    const EXPECTED: u32 = {
        const OFFSET: u32 = 0x811c_9dc5;
        const PRIME: u32 = 0x0100_0193;
        let bytes = b"liblv2";
        let mut h = OFFSET;
        let mut i = 0;
        while i < bytes.len() {
            h ^= bytes[i] as u32;
            h = h.wrapping_mul(PRIME);
            i += 1;
        }
        h
    };
    assert_eq!(graph::module_id_from_name("liblv2"), PrxModuleId(EXPECTED));
}

fn one_import(nid: u32, stub_addr: u32) -> Vec<crate::prx::ImportedModule> {
    vec![crate::prx::ImportedModule {
        name: "synth".to_string(),
        functions: vec![crate::prx::ImportedFunction { nid, stub_addr }],
        variables: Vec::new(),
    }]
}

#[test]
fn patch_imports_against_unresolved_nid_yields_unresolved_import() {
    let table = FirmwareExportTable::default(); // empty
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    let err =
        patch_imports_against(&one_import(0xDEADBEEF, 0x100), &table, 0, &mut mem).unwrap_err();
    assert_eq!(err, PrxLoaderError::UnresolvedImport { nid: 0xDEADBEEF });
}

#[test]
fn patch_imports_against_opd_above_u32_yields_out_of_range() {
    let table = FirmwareExportTable::for_test(&[(0xCAFEBABE, 0x1_0000_0000u64)]);
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    let err =
        patch_imports_against(&one_import(0xCAFEBABE, 0x100), &table, 0, &mut mem).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::OpdAddressOutOfRange {
            nid: 0xCAFEBABE,
            addr: 0x1_0000_0000,
        }
    );
}

#[test]
fn patch_imports_against_succeeds_and_writes_be_opd_into_got_slot() {
    let table = FirmwareExportTable::for_test(&[(0xAAAA1111, 0x4000_0080u64)]);
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    patch_imports_against(&one_import(0xAAAA1111, 0x100), &table, 0, &mut mem).expect("patch");
    let got = &mem.as_bytes()[0x100..0x104];
    assert_eq!(got, &0x4000_0080u32.to_be_bytes());
}

#[test]
fn patch_imports_against_writes_at_load_base_plus_stub_addr() {
    // Firmware PRXs parse with PIC-base-0 vaddrs; patch fires at
    // `load_base + stub_addr`.
    let opd_addr: u64 = 0x4000_0080;
    let load_base: u64 = 0x2000;
    let stub_vaddr: u32 = 0x300;
    let runtime_stub = load_base + u64::from(stub_vaddr);
    let table = FirmwareExportTable::for_test(&[(0xAAAA1111, opd_addr)]);
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    patch_imports_against(
        &one_import(0xAAAA1111, stub_vaddr),
        &table,
        load_base,
        &mut mem,
    )
    .expect("patch");
    assert_eq!(
        &mem.as_bytes()[runtime_stub as usize..runtime_stub as usize + 4],
        &(opd_addr as u32).to_be_bytes(),
    );
    assert_eq!(
        &mem.as_bytes()[stub_vaddr as usize..stub_vaddr as usize + 4],
        &[0u8; 4],
        "load_base = 0x2000 should redirect the write away from vaddr 0x300"
    );
}

#[test]
fn patch_imports_against_is_atomic_on_phase1_failure() {
    // First import resolves, second is missing; the failure must
    // discard the first.
    let table = FirmwareExportTable::for_test(&[(0xAAAA1111, 0x4000_0080u64)]);
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    let before = mem.content_hash();
    let imports = vec![crate::prx::ImportedModule {
        name: "synth".to_string(),
        functions: vec![
            crate::prx::ImportedFunction {
                nid: 0xAAAA1111,
                stub_addr: 0x100,
            },
            crate::prx::ImportedFunction {
                nid: 0xBBBB2222,
                stub_addr: 0x110,
            },
        ],
        variables: Vec::new(),
    }];
    let err = patch_imports_against(&imports, &table, 0, &mut mem).unwrap_err();
    assert_eq!(err, PrxLoaderError::UnresolvedImport { nid: 0xBBBB2222 });
    assert_eq!(
        mem.content_hash(),
        before,
        "Phase-1 failure committed bytes: atomic-batch violated"
    );
}

#[test]
fn patch_imports_against_is_atomic_on_phase2_drain_failure() {
    // Both imports resolve in Phase-1; the second points outside
    // GuestMemory so the Phase-2 drain rejects the batch.
    let table = FirmwareExportTable::for_test(&[
        (0xAAAA1111, 0x4000_0080u64),
        (0xBBBB2222, 0x4000_00C0u64),
    ]);
    let mut mem = cellgov_mem::GuestMemory::new(0x1000);
    let before = mem.content_hash();
    let imports = vec![crate::prx::ImportedModule {
        name: "synth".to_string(),
        functions: vec![
            crate::prx::ImportedFunction {
                nid: 0xAAAA1111,
                stub_addr: 0x100,
            },
            crate::prx::ImportedFunction {
                nid: 0xBBBB2222,
                stub_addr: 0xFFFF_0000,
            },
        ],
        variables: Vec::new(),
    }];
    let err = patch_imports_against(&imports, &table, 0, &mut mem).unwrap_err();
    match err {
        PrxLoaderError::GotBatchPatchFailed { count, source: _ } => {
            assert_eq!(count, 2, "batch carries the full staged count");
        }
        other => panic!("expected GotBatchPatchFailed, got {other:?}"),
    }
    assert_eq!(
        mem.content_hash(),
        before,
        "Phase-2 drain failure committed bytes: atomic-batch violated"
    );
}

#[test]
fn load_firmware_set_rejects_duplicate_module_id_and_does_not_touch_memory() {
    let bytes = crate::sprx::test_fixtures::make_test_prx();
    let mut by_path = BTreeMap::new();
    by_path.insert("alpha.sprx".to_string(), bytes.clone());
    by_path.insert("beta.sprx".to_string(), bytes);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let before = mem.content_hash();
    let err = load_firmware_set(by_path, &mut mem, 0x1000_0000).unwrap_err();
    match err {
        PrxLoaderError::DuplicateModuleId {
            id: _,
            first_path,
            second_path,
        } => {
            // BTreeMap iteration is lexicographic by key: alpha
            // is first, beta is second.
            assert_eq!(first_path, "alpha.sprx");
            assert_eq!(second_path, "beta.sprx");
        }
        other => panic!("expected DuplicateModuleId, got {other:?}"),
    }
    assert_eq!(
        mem.content_hash(),
        before,
        "DuplicateModuleId fired but memory was mutated: dedup check must run before load_prx"
    );
}

#[test]
fn patch_imports_against_empty_import_list_is_noop_ok() {
    let table = FirmwareExportTable::default();
    let mut mem = cellgov_mem::GuestMemory::new(0x10_000);
    let before = mem.content_hash();
    patch_imports_against(&[], &table, 0, &mut mem).expect("patch");
    assert_eq!(mem.content_hash(), before);
}

// -- Export-namespace identity --

/// Add a single synthetic import entry to `make_test_prx`'s bytes
/// declaring an import of one NID from namespace `imp_name`. The
/// entry is placed in segment 1 (data) past the existing layout
/// at file offset 0x300; vaddr 0x210 in segment 1. The fixture's
/// library_info imports_start/end and the import-table entry's
/// name/nid/stub pointers are patched accordingly.
fn make_test_prx_importing(imp_name: &str) -> Vec<u8> {
    let mut data = crate::sprx::test_fixtures::make_test_prx();
    // Entry at file 0x300 (vaddr 0x210); 0x2C bytes; one function.
    let entry_off: usize = 0x300;
    let imp_name_off: usize = entry_off + 0x30; // file 0x330, vaddr 0x240
    let imp_nid_off: usize = entry_off + 0x50; // file 0x350, vaddr 0x260
    let imp_stub_off: usize = entry_off + 0x60; // file 0x360, vaddr 0x270

    // library_info imports_start/end (file 0x1F0 + 44/48 = 0x21C/0x220):
    // entry begins at vaddr 0x210, ends at vaddr 0x210 + 0x2C = 0x23C.
    let mi = 0x1F0usize;
    data[mi + 44..mi + 48].copy_from_slice(&0x210u32.to_be_bytes());
    data[mi + 48..mi + 52].copy_from_slice(&0x23Cu32.to_be_bytes());

    // PrxImportEntry @ entry_off (vaddr 0x210):
    // size=0x2C, num_func=1, name_ptr/nid_ptr/stub_ptr.
    data[entry_off] = 0x2C;
    data[entry_off + 6..entry_off + 8].copy_from_slice(&1u16.to_be_bytes());
    // name_ptr (vaddr 0x240)
    data[entry_off + 16..entry_off + 20].copy_from_slice(&0x240u32.to_be_bytes());
    // nid_ptr (vaddr 0x260)
    data[entry_off + 20..entry_off + 24].copy_from_slice(&0x260u32.to_be_bytes());
    // stub_ptr (vaddr 0x270)
    data[entry_off + 24..entry_off + 28].copy_from_slice(&0x270u32.to_be_bytes());

    // Write the import-module name (NUL-terminated).
    let name_bytes = imp_name.as_bytes();
    assert!(
        name_bytes.len() < 32,
        "test fixture: name too long for 0x20-byte region"
    );
    data[imp_name_off..imp_name_off + name_bytes.len()].copy_from_slice(name_bytes);
    data[imp_name_off + name_bytes.len()] = 0;

    // One NID + stub slot.
    data[imp_nid_off..imp_nid_off + 4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    data[imp_stub_off..imp_stub_off + 4].copy_from_slice(&0u32.to_be_bytes());

    data
}

#[test]
fn load_firmware_set_missing_namespace_reports_namespace_id() {
    let bytes = make_test_prx_importing("ghostlib");
    let mut by_path = BTreeMap::new();
    by_path.insert("solo.sprx".to_string(), bytes);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let err = load_firmware_set(by_path, &mut mem, 0x1000_0000).unwrap_err();
    match err {
        PrxLoaderError::MissingDependency { target, .. } => {
            let expected = graph::module_id_from_name("ghostlib");
            assert_eq!(
                target, expected,
                "MissingDependency.target must be the namespace id, \
                 not the file's library_info-name id"
            );
        }
        other => panic!("expected MissingDependency, got {other:?}"),
    }
}

#[test]
fn load_firmware_set_self_namespace_import_does_not_trip_missing_dependency() {
    // make_test_prx exports under "testlib"; importing that
    // namespace from itself must not trip MissingDependency.
    let bytes = make_test_prx_importing("testlib");
    let mut by_path = BTreeMap::new();
    by_path.insert("solo.sprx".to_string(), bytes);
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let result = load_firmware_set(by_path, &mut mem, 0x1000_0000);
    if let Err(PrxLoaderError::MissingDependency { target, .. }) = &result {
        panic!(
            "self-namespace import tripped MissingDependency (target={target:?}); \
             expected the loader to recognise testlib's own export"
        );
    }
}
