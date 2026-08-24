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
    assert_eq!(d.lengths, Some((2, 3)));
}

/// The walk reads a short side as zeros past its end, so a surplus of
/// zeros produces no differing byte. Two sides that disagree on how
/// much of a region exists have still diverged.
#[test]
fn a_zero_only_length_surplus_still_diverges() {
    for (exp_data, act_data, want) in [
        (vec![1, 2], vec![1, 2, 0, 0], (2, 4)),
        (vec![1, 2, 0, 0], vec![1, 2], (4, 2)),
        (Vec::new(), vec![0], (0, 1)),
    ] {
        let exp = vec![region("r", exp_data)];
        let act = vec![region("r", act_data)];
        let d = find_memory_divergence(&exp, &act)
            .unwrap_or_else(|| panic!("lengths {want:?} compared as a match"));
        assert_eq!(d.region, "r");
        assert_eq!(d.lengths, Some(want));
        assert_eq!(d.offset, want.0.min(want.1));
    }
}

#[test]
fn equal_length_matching_regions_report_no_divergence() {
    let exp = vec![region("r", vec![1, 2, 0, 0])];
    let act = vec![region("r", vec![1, 2, 0, 0])];
    assert_eq!(find_memory_divergence(&exp, &act), None);
}

/// A byte difference earlier than the length disagreement is the more
/// useful report, so it wins -- but the lengths ride along.
#[test]
fn a_byte_difference_is_reported_ahead_of_a_length_difference() {
    let exp = vec![region("r", vec![1, 9])];
    let act = vec![region("r", vec![1, 2, 3])];
    let d = find_memory_divergence(&exp, &act).expect("diverges");
    assert_eq!(d.offset, 1);
    assert_eq!((d.expected, d.actual), (9, 2));
    assert_eq!(d.lengths, Some((2, 3)));
}
