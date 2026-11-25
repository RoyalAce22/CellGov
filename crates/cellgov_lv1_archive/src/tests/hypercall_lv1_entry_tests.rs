//! Each number's `lv1_493` cell names its entry in lv1's live 4.93
//! hypercall table.

use cellgov_ps3_abi::lv1::table::{HYPERCALL_TABLE_ENTRIES, LIVE_TABLE_HANDLERS};

use super::Lv1Entry;

#[test]
fn a_number_takes_the_label_of_its_live_4_93_entry() {
    let has_handler = |number: u64| {
        LIVE_TABLE_HANDLERS
            .iter()
            .any(|(n, _)| u64::from(*n) == number)
    };
    let handler = u64::from(
        LIVE_TABLE_HANDLERS
            .last()
            .expect("the live table has a handler")
            .0,
    );
    let invalid = (0..HYPERCALL_TABLE_ENTRIES)
        .find(|n| !has_handler(*n))
        .expect("a number below the table end holds the Invalid HV Call function");
    assert_eq!(Lv1Entry::of(handler), Lv1Entry::Handler);
    assert_eq!(Lv1Entry::of(invalid), Lv1Entry::Invalid);
    assert_eq!(Lv1Entry::of(HYPERCALL_TABLE_ENTRIES), Lv1Entry::PastTable);
    let labels: Vec<&str> = [Lv1Entry::Handler, Lv1Entry::Invalid, Lv1Entry::PastTable]
        .iter()
        .map(|e| e.label())
        .collect();
    assert_eq!(labels, crate::spec::HYPERCALL_LV1_ENTRIES);
}
