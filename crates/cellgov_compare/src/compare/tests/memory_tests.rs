//! Memory-region divergence localized to the first differing byte, missing region, or length mismatch.

use super::*;
use crate::test_support::region;

#[test]
fn memory_divergence_reports_first_differing_byte() {
    let exp = vec![region("r", vec![1, 2, 3])];
    let act = vec![region("r", vec![1, 2, 99])];
    let d = find_memory_divergence(&exp, &act).expect("diverges");
    assert_eq!(d.region, "r");
    assert_eq!(d.offset, 2);
    assert_eq!(d.expected, 3);
    assert_eq!(d.actual, 99);
}

#[test]
fn missing_memory_region_is_divergence() {
    let exp = vec![region("r", vec![1])];
    let act = vec![];
    let d = find_memory_divergence(&exp, &act).expect("diverges");
    assert_eq!(d.region, "r");
}

#[test]
fn extra_memory_region_in_actual_is_divergence() {
    let exp = vec![];
    let act = vec![region("extra", vec![1])];
    let d = find_memory_divergence(&exp, &act).expect("diverges");
    assert_eq!(d.region, "extra");
}

#[test]
fn different_length_memory_regions_diverge() {
    let exp = vec![region("r", vec![1, 2])];
    let act = vec![region("r", vec![1, 2, 3])];
    let d = find_memory_divergence(&exp, &act).expect("diverges");
    assert_eq!(d.offset, 2);
    assert_eq!(d.expected, 0);
    assert_eq!(d.actual, 3);
}
