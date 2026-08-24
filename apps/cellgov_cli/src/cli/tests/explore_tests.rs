//! Oracle-comparison guards for `explore micro --observations-dir`.

use super::*;
use cellgov_explore::oracle::{CapturedRegion, ScheduleSnapshot};

fn region(name: &str, data: &[u8], resolved: bool) -> CapturedRegion {
    CapturedRegion {
        name: name.to_string(),
        data: data.to_vec(),
        resolved,
    }
}

fn snapshot(regions: Vec<CapturedRegion>) -> ScheduleSnapshot {
    ScheduleSnapshot {
        memory_hash: 0,
        regions,
    }
}

#[test]
fn every_capture_resolving_reports_nothing_unresolved() {
    let baseline = snapshot(vec![region("result", &[1, 2, 3], true)]);
    let alternates = vec![snapshot(vec![region("result", &[1, 2, 3], true)])];
    assert!(unresolved_region_names(&baseline, &alternates).is_empty());
}

#[test]
fn an_unresolved_capture_is_named_by_schedule_and_region() {
    // An unresolved capture carries empty bytes whatever the run did,
    // so leaving it in the comparison reports a harness fault as an
    // oracle mismatch.
    let baseline = snapshot(vec![region("header", &[], false)]);
    let alternates = vec![
        snapshot(vec![region("header", &[9], true)]),
        snapshot(vec![region("data", &[], false)]),
    ];
    assert_eq!(
        unresolved_region_names(&baseline, &alternates),
        vec!["baseline:header".to_string(), "schedule 1:data".to_string()]
    );
}

#[test]
fn an_empty_but_resolved_capture_is_not_unresolved() {
    // A zero-size spec legitimately reads back no bytes; only the
    // `resolved` flag separates that from a range that could not be
    // read at all.
    let baseline = snapshot(vec![region("empty", &[], true)]);
    assert!(unresolved_region_names(&baseline, &[]).is_empty());
}
