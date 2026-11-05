//! The load profile's per-site history and its summary.

use super::{LoadOutcome, LoadProfile, LoadSummary};

#[test]
fn a_site_that_rereads_one_view_counts_each_reread_as_the_same() {
    let mut p = LoadProfile::default();
    for _ in 0..3 {
        p.record(Some(0x100), LoadOutcome::Region(2));
    }
    let s = p.summary();
    assert_eq!(s.region_loads, 3);
    assert_eq!(s.same_as_previous, 2);
    assert_eq!(s.sites_by_regions, [1, 0, 0]);
    assert_eq!(s.loads_by_regions, [3, 0, 0]);
}

#[test]
fn a_site_that_alternates_views_lands_in_the_two_view_bucket() {
    let mut p = LoadProfile::default();
    for index in [0, 1, 0, 1] {
        p.record(Some(0x100), LoadOutcome::Region(index));
    }
    for index in [0, 1, 2] {
        p.record(Some(0x200), LoadOutcome::Region(index));
    }
    let s = p.summary();
    assert_eq!(s.same_as_previous, 0);
    assert_eq!(s.sites, 2);
    assert_eq!(s.sites_by_regions, [0, 1, 1]);
    assert_eq!(s.loads_by_regions, [0, 4, 3]);
}

#[test]
fn forwarded_unheld_and_unslotted_loads_join_no_site() {
    let mut p = LoadProfile::default();
    p.record(Some(0x100), LoadOutcome::Forwarded);
    p.record(Some(0x100), LoadOutcome::Unheld);
    p.record(None, LoadOutcome::Region(0));
    p.record(None, LoadOutcome::Forwarded);
    let s = p.summary();
    assert_eq!((s.forwarded, s.unheld, s.no_shadow), (2, 1, 1));
    assert_eq!((s.sites, s.region_loads), (0, 0));
    assert_eq!(p.sites().count(), 0);
}

#[test]
fn only_a_layout_that_differs_from_the_last_counts_as_a_change() {
    let mut p = LoadProfile::default();
    let a = [(0, 0x1000), (0x10000, 0x1000)];
    let b = [(0, 0x1000), (0x10000, 0x800), (0x10800, 0x800)];
    p.note_layout(a.into_iter());
    p.note_layout(a.into_iter());
    assert_eq!(
        p.layout_changes, 0,
        "the first layout and a repeat change nothing"
    );
    p.note_layout(b.into_iter());
    p.note_layout(a.into_iter());
    assert_eq!(p.layout_changes, 2);
}

#[test]
fn merged_summaries_add_field_by_field() {
    let mut p = LoadProfile::default();
    p.record(Some(4), LoadOutcome::Region(0));
    p.record(Some(4), LoadOutcome::Region(0));
    p.record(Some(8), LoadOutcome::Forwarded);
    let one = p.summary();
    let mut two = one;
    two.merge(&one);
    assert_eq!(
        two,
        LoadSummary {
            forwarded: 2,
            region_loads: 4,
            same_as_previous: 2,
            sites: 2,
            sites_by_regions: [2, 0, 0],
            loads_by_regions: [4, 0, 0],
            ..LoadSummary::default()
        }
    );
    assert!(two.to_json().contains("\"loads_by_regions\": [4, 0, 0]"));
}

/// A summary whose every count differs from every other.
fn distinct(scale: u64) -> LoadSummary {
    LoadSummary {
        forwarded: scale,
        region_loads: 2 * scale,
        same_as_previous: 3 * scale,
        unheld: 4 * scale,
        no_shadow: 5 * scale,
        unkeyed_fused: 6 * scale,
        layout_changes: 7 * scale,
        sites: 8 * scale,
        sites_by_regions: [9 * scale, 10 * scale, 11 * scale],
        loads_by_regions: [12 * scale, 13 * scale, 14 * scale],
    }
}

#[test]
fn a_merge_adds_each_field_of_the_other_summary_to_the_same_field() {
    let mut sum = distinct(1);
    sum.merge(&distinct(100));
    assert_eq!(sum, distinct(101));
}

#[test]
fn the_json_names_each_count_by_its_own_field() {
    assert_eq!(
        distinct(1).to_json(),
        "{\n  \"forwarded\": 1,\n  \"region_loads\": 2,\n  \"same_as_previous\": 3,\n  \
         \"unheld\": 4,\n  \"no_shadow\": 5,\n  \"unkeyed_fused\": 6,\n  \
         \"layout_changes\": 7,\n  \"sites\": 8,\n  \
         \"sites_by_regions\": [9, 10, 11],\n  \"loads_by_regions\": [12, 13, 14]\n}\n"
    );
}
