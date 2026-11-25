//! Each number's `lv1_493` cell names its entry in lv1's live 4.93
//! hypercall table.

use super::Lv1Entry;

#[test]
fn a_number_takes_the_label_of_its_live_4_93_entry() {
    assert_eq!(Lv1Entry::of(246), Lv1Entry::Handler);
    assert_eq!(Lv1Entry::of(14), Lv1Entry::Invalid);
    assert_eq!(Lv1Entry::of(256), Lv1Entry::PastTable);
    let labels: Vec<&str> = [Lv1Entry::Handler, Lv1Entry::Invalid, Lv1Entry::PastTable]
        .iter()
        .map(|e| e.label())
        .collect();
    assert_eq!(labels, crate::spec::HYPERCALL_LV1_ENTRIES);
}
