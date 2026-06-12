//! Stub-classification coverage and nid_db lookup across HLE module NID inventories.

use super::*;

#[test]
fn stub_class_as_str_round_trips_inventory_labels() {
    assert_eq!(StubClass::NoopSafe.as_str(), "noop-safe");
    assert_eq!(StubClass::Stateful.as_str(), "stateful");
    assert_eq!(StubClass::UnsafeToStub.as_str(), "unsafe-to-stub");
}

#[test]
fn classification_covers_user_listed_misclassifications() {
    assert_eq!(stub_classification(0x35168520), StubClass::UnsafeToStub); // _sys_heap_malloc
    assert_eq!(stub_classification(0x44265c08), StubClass::UnsafeToStub); // _sys_heap_memalign
    assert_eq!(stub_classification(0xb2fcf2c8), StubClass::Stateful); // _sys_heap_create_heap
    assert_eq!(stub_classification(0x2f85c0ef), StubClass::Stateful); // sys_lwmutex_create
    assert_eq!(stub_classification(0xf7f7fb20), StubClass::NoopSafe); // _sys_free
}

#[test]
fn lookup_returns_empty_module_for_libstdcxx_symbols() {
    let (m, n) = lookup(0x003395d9).expect("_Feraise is in nid_db");
    assert_eq!(m, "");
    assert_eq!(n, "_Feraise");
}

/// Every NID listed in any module's `CLASSIFIED_NIDS` slice must
/// classify explicitly (not fall to the default `NoopSafe`
/// catch-all). The default arm of `stub_classification` is
/// "unreviewed"; reaching it from a slice-listed NID is a
/// documentation/reviewer gap. This is a DIAGNOSTIC contract
/// (the classifier is HLE-as-tooling, not a dispatch table); the
/// `CLASSIFIED_NIDS` slices are the classifier's
/// NID-set-of-record.
///
/// `cell_save_data`'s `CLASSIFIED_NIDS` carries only `AUTO_LOAD`
/// and `AUTO_LOAD_2`; the `AUTO_SAVE` / `AUTO_SAVE_2` /
/// `LIST_AUTO_LOAD` NIDs declared in that module sit outside the
/// slice (see the module docstring) so they are not in scope
/// for this check.
#[test]
fn every_classified_nid_has_explicit_arm() {
    let classified_slices: &[(&str, &[u32])] = &[
        ("sys_prx_for_user", sys_prx_for_user::CLASSIFIED_NIDS),
        ("sys_fs", sys_fs::CLASSIFIED_NIDS),
        ("cell_sysutil", cell_sysutil::CLASSIFIED_NIDS),
        ("cell_save_data", cell_save_data::CLASSIFIED_NIDS),
        ("cell_gcm_sys", cell_gcm_sys::CLASSIFIED_NIDS),
        ("cell_spurs", cell_spurs::CLASSIFIED_NIDS),
    ];
    for (module, classified) in classified_slices {
        for &nid in *classified {
            assert!(
                stub_classification_explicit(nid).is_some(),
                "{module}::CLASSIFIED_NIDS NID 0x{nid:08x} fell to the \
                 default (presumptive-NoopSafe) arm; add an explicit \
                 classification arm or remove it from CLASSIFIED_NIDS",
            );
        }
    }
}
