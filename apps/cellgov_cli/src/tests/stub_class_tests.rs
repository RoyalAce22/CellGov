use super::*;
use cellgov_ps3_abi::nid::{cell_save_data, CURATED};

/// Curated NIDs with no recorded verdict; each prints as `noop-safe`
/// by presumption.
const UNREVIEWED: &[u32] = &[
    cell_save_data::AUTO_SAVE,
    cell_save_data::AUTO_SAVE_2,
    cell_save_data::LIST_AUTO_LOAD,
];

#[test]
fn as_str_round_trips_the_inventory_labels() {
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
fn an_unreviewed_nid_is_presumed_noop_safe_and_reads_as_unreviewed() {
    let nid = cell_save_data::AUTO_SAVE;
    assert_eq!(reviewed_stub_classification(nid), None);
    assert_eq!(stub_classification(nid), StubClass::NoopSafe);
}

#[test]
fn a_reviewed_noop_safe_nid_reads_as_reviewed() {
    let nid = cellgov_ps3_abi::nid::sys_prx_for_user::FREE;
    assert_eq!(reviewed_stub_classification(nid), Some(StubClass::NoopSafe));
}

#[test]
fn every_curated_nid_is_reviewed_unless_listed_unreviewed() {
    let mut wrong = Vec::new();
    let mut checked = 0usize;
    for (module, declared) in CURATED {
        for &(nid, name) in *declared {
            checked += 1;
            let reviewed = reviewed_stub_classification(nid).is_some();
            let listed = UNREVIEWED.contains(&nid);
            match (reviewed, listed) {
                (true, true) => wrong.push(format!(
                    "{module}::{name} 0x{nid:08x} has a verdict; drop it from UNREVIEWED"
                )),
                (false, false) => wrong.push(format!(
                    "{module}::{name} 0x{nid:08x} has no verdict; record one or list it in UNREVIEWED"
                )),
                _ => {}
            }
        }
    }
    assert!(checked > UNREVIEWED.len(), "CURATED holds no reviewed NID");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_unreviewed_nid_is_curated() {
    for &nid in UNREVIEWED {
        assert!(
            CURATED
                .iter()
                .any(|(_, declared)| declared.iter().any(|&(n, _)| n == nid)),
            "0x{nid:08x} in UNREVIEWED is not a curated NID"
        );
    }
}
