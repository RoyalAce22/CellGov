//! FirmwareExportTable construction and NID lookup across loaded modules.

use super::*;
use crate::sprx::{LoadedOpd, LoadedPrx};

fn loaded_stub(module_id: PrxModuleId, exports: &[(u32, u64)]) -> LoadedPrx {
    LoadedPrx {
        name: format!("m{}", module_id.0),
        module_id,
        base: 0,
        toc: 0,
        text_start: 0,
        text_end: 0,
        data_start: 0,
        data_end: 0,
        exports: exports.iter().copied().collect(),
        module_start: None::<LoadedOpd>,
        module_stop: None::<LoadedOpd>,
        relocs_applied: 0,
    }
}

fn loaded_set(items: &[(PrxModuleId, &[(u32, u64)])]) -> BTreeMap<PrxModuleId, LoadedPrx> {
    items
        .iter()
        .map(|(id, exports)| (*id, loaded_stub(*id, exports)))
        .collect()
}

#[test]
fn build_empty_table_from_no_modules() {
    let loaded: BTreeMap<PrxModuleId, LoadedPrx> = BTreeMap::new();
    let t = FirmwareExportTable::build(&loaded, &[]).expect("build");
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
}

#[test]
fn build_single_module_table_lists_every_export() {
    let id = PrxModuleId(1);
    let loaded = loaded_set(&[(id, &[(0xA, 0x1000), (0xB, 0x2000)])]);
    let t = FirmwareExportTable::build(&loaded, &[id]).expect("build");
    assert_eq!(t.get(0xA), Some(0x1000));
    assert_eq!(t.get(0xB), Some(0x2000));
    assert_eq!(t.get(0xC), None);
}

#[test]
fn build_multi_module_table_unions_disjoint_exports() {
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xB, 0x2000)])]);
    let t = FirmwareExportTable::build(&loaded, &[a, b]).expect("build");
    assert_eq!(t.get(0xA), Some(0x1000));
    assert_eq!(t.get(0xB), Some(0x2000));
    let nids: BTreeSet<u32> = t.nids().collect();
    assert_eq!(nids, [0xA, 0xB].into_iter().collect());
}

#[test]
fn build_silently_accepts_same_nid_same_opd() {
    // Defensive case: not produced by shipping SPRX (exports
    // point into the exporter's own text), but if it ever
    // happens, agreement on the same OPD is a no-op.
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xA, 0x1000)])]);
    let t = FirmwareExportTable::build(&loaded, &[a, b]).expect("build");
    assert_eq!(t.get(0xA), Some(0x1000));
}

#[test]
fn build_rejects_same_nid_different_opd() {
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[(0xA, 0x1000)]), (b, &[(0xA, 0x2000)])]);
    let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
    let PrxLoaderError::ConflictingExport { nid, first, second } = err else {
        panic!("expected ConflictingExport");
    };
    assert_eq!(nid, 0xA);
    assert_eq!(first, a);
    assert_eq!(second, b);
}

#[test]
fn build_iteration_is_deterministic_across_two_builds() {
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[(0xA, 0x1000), (0xC, 0x3000)]), (b, &[(0xB, 0x2000)])]);
    let t1 = FirmwareExportTable::build(&loaded, &[a, b]).expect("build1");
    let t2 = FirmwareExportTable::build(&loaded, &[a, b]).expect("build2");
    let k1: Vec<u32> = t1.nids().collect();
    let k2: Vec<u32> = t2.nids().collect();
    assert_eq!(k1, k2);
}

#[test]
fn build_rejects_module_in_order_missing_from_loaded() {
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[])]); // only a loaded
    let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
    let PrxLoaderError::OrderLoadedMismatch {
        in_order_not_loaded,
        in_loaded_not_order,
    } = err
    else {
        panic!("expected OrderLoadedMismatch");
    };
    assert_eq!(in_order_not_loaded, vec![b]);
    assert!(in_loaded_not_order.is_empty());
}

#[test]
fn build_rejects_module_in_loaded_missing_from_order() {
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[(a, &[]), (b, &[])]);
    let err = FirmwareExportTable::build(&loaded, &[a]).unwrap_err();
    let PrxLoaderError::OrderLoadedMismatch {
        in_order_not_loaded,
        in_loaded_not_order,
    } = err
    else {
        panic!("expected OrderLoadedMismatch");
    };
    assert!(in_order_not_loaded.is_empty());
    assert_eq!(in_loaded_not_order, vec![b]);
}

#[test]
fn build_rejects_duplicate_id_in_order() {
    let a = PrxModuleId(1);
    let loaded = loaded_set(&[(a, &[])]);
    let err = FirmwareExportTable::build(&loaded, &[a, a]).unwrap_err();
    assert_eq!(
        err,
        PrxLoaderError::DuplicateModuleInOrder {
            id: a,
            first_index: 0,
            second_index: 1,
        }
    );
}

#[test]
fn build_reports_first_recorder_on_three_way_conflict() {
    // A records 0xA at 0x1000; B agrees (no-op); C disagrees.
    // The error must name A as `first` (the recorder), not B
    // (the silent agreer). Locking the recorder semantics.
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let c = PrxModuleId(3);
    let loaded = loaded_set(&[
        (a, &[(0xA, 0x1000)]),
        (b, &[(0xA, 0x1000)]),
        (c, &[(0xA, 0x2000)]),
    ]);
    let err = FirmwareExportTable::build(&loaded, &[a, b, c]).unwrap_err();
    let PrxLoaderError::ConflictingExport { nid, first, second } = err else {
        panic!("expected ConflictingExport");
    };
    assert_eq!(nid, 0xA);
    assert_eq!(first, a, "recorder is the first writer, not the agreer");
    assert_eq!(second, c);
}

#[test]
fn build_returns_on_first_conflict_when_multiple_exist() {
    // A records 0xA at 0x1000 and 0xB at 0x2000. B disagrees
    // on BOTH. build must early-return on the first conflict
    // it walks (0xA, since BTreeMap export iteration is sorted)
    // and not surface the second.
    let a = PrxModuleId(1);
    let b = PrxModuleId(2);
    let loaded = loaded_set(&[
        (a, &[(0xA, 0x1000), (0xB, 0x2000)]),
        (b, &[(0xA, 0x9000), (0xB, 0x9000)]),
    ]);
    let err = FirmwareExportTable::build(&loaded, &[a, b]).unwrap_err();
    let PrxLoaderError::ConflictingExport { nid, .. } = err else {
        panic!("expected ConflictingExport");
    };
    assert_eq!(
        nid, 0xA,
        "early-return must surface the first conflict, not the last"
    );
}

impl FirmwareExportTable {
    /// Test-only constructor: build a table directly from
    /// (nid, opd) pairs with a synthetic origin. Lets unit tests
    /// inject a known table without paying the precondition checks
    /// `build` runs over `loaded` / `order`.
    pub(crate) fn for_test(entries: &[(u32, u64)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|&(nid, opd)| (nid, (opd, PrxModuleId(0))))
                .collect(),
        }
    }
}
