use std::collections::BTreeMap;

use crate::boot_history::{parse, render_line, BootHistoryEntry};
use crate::witness_parse::{known_witness_names, line_of, parse_witness_lines, ParsedWitnesses};
use crate::witnesses::{
    check_all, record, unrecorded, Witness, WitnessClass, WitnessFailure, WitnessSet,
};

fn obs(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

/// Observation whose `seen_lines` covers every named witness, so the
/// class checks are exercised rather than the line-absence arm.
fn parsed(pairs: &[(&str, u64)]) -> ParsedWitnesses {
    ParsedWitnesses {
        values: obs(pairs),
        seen_lines: pairs.iter().filter_map(|(k, _)| line_of(k)).collect(),
    }
}

fn baseline(pairs: &[(&str, u64, WitnessClass)]) -> WitnessSet {
    pairs
        .iter()
        .map(|(k, v, c)| {
            (
                (*k).to_string(),
                Witness {
                    value: *v,
                    class: *c,
                },
            )
        })
        .collect()
}

#[test]
fn exact_rejects_any_movement() {
    let b = baseline(&[("host_invariant_breaks", 43, WitnessClass::Exact)]);
    assert!(check_all(&b, &parsed(&[("host_invariant_breaks", 43)])).is_empty());
    assert_eq!(
        check_all(&b, &parsed(&[("host_invariant_breaks", 44)])).len(),
        1
    );
    assert_eq!(
        check_all(&b, &parsed(&[("host_invariant_breaks", 42)])).len(),
        1
    );
}

#[test]
fn at_least_tolerates_growth_and_rejects_loss() {
    let b = baseline(&[("ldarx", 100, WitnessClass::AtLeast)]);
    assert!(check_all(&b, &parsed(&[("ldarx", 100)])).is_empty());
    assert!(check_all(&b, &parsed(&[("ldarx", 5000)])).is_empty());
    assert_eq!(check_all(&b, &parsed(&[("ldarx", 99)])).len(), 1);
}

#[test]
fn absent_rejects_a_path_that_started_firing() {
    let b = baseline(&[("dcbz", 0, WitnessClass::Absent)]);
    assert!(check_all(&b, &parsed(&[("dcbz", 0)])).is_empty());
    assert_eq!(check_all(&b, &parsed(&[("dcbz", 1)])).len(), 1);
}

#[test]
fn informational_never_fails() {
    let b = baseline(&[("dcbz", 100, WitnessClass::Informational)]);
    assert!(check_all(&b, &parsed(&[("dcbz", 999_999)])).is_empty());
    assert!(check_all(&b, &parsed(&[("dcbz", 0)])).is_empty());
}

/// A witness whose emitting line vanishes must fail as line-absence,
/// not read as zero.
#[test]
fn a_recorded_witness_that_vanishes_is_a_line_absence_failure() {
    let b = baseline(&[("ldarx", 100, WitnessClass::AtLeast)]);
    let failures = check_all(&b, &parsed(&[]));
    assert_eq!(failures.len(), 1);
    assert!(matches!(&failures[0], WitnessFailure::LineAbsent { .. }));
}

/// The vacuity hole this arm closes: an `Absent`-class witness reads
/// observed 0 whether its line reported 0 or was never emitted at
/// all. Deleting the emitter must fail, not stay green.
#[test]
fn an_absent_class_witness_fails_when_its_line_is_never_emitted() {
    let b = baseline(&[("dcbz", 0, WitnessClass::Absent)]);
    assert!(
        check_all(&b, &parsed(&[("dcbz", 0)])).is_empty(),
        "line emitted, value 0: passes"
    );
    let failures = check_all(&b, &parsed(&[]));
    assert_eq!(failures.len(), 1, "line never emitted: fails");
    assert!(matches!(&failures[0], WitnessFailure::LineAbsent { .. }));
}

#[test]
fn a_baseline_witness_unknown_to_this_build_fails() {
    let b = baseline(&[("renamed_away", 3, WitnessClass::AtLeast)]);
    let failures = check_all(&b, &parsed(&[]));
    assert_eq!(failures.len(), 1);
    assert!(matches!(
        &failures[0],
        WitnessFailure::UnknownWitness { .. }
    ));
}

#[test]
fn a_duplicate_bench_line_is_a_parse_error() {
    let errs = parse_witness_lines("BENCH_DCBZ_WITNESS: count=1\nBENCH_DCBZ_WITNESS: count=2\n")
        .expect_err("a duplicate line must not silently last-write-win");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].to_string().contains("more than once"));
}

#[test]
fn an_unknown_key_on_a_tracked_line_is_a_parse_error() {
    let errs = parse_witness_lines("BENCH_PRX_LOAD_WITNESS: hle_stubs=1 missing=2\n")
        .expect_err("a renamed emitter token must fail, not drop the witness");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].to_string().contains("missing"));
}

#[test]
fn unrecorded_names_witnesses_the_baseline_lacks() {
    let b = baseline(&[("ldarx", 1, WitnessClass::AtLeast)]);
    assert_eq!(
        unrecorded(&b, &obs(&[("ldarx", 1), ("brand_new", 7)])),
        vec!["brand_new"]
    );
    assert!(unrecorded(&b, &obs(&[("ldarx", 1)])).is_empty());
}

#[test]
fn first_record_classifies_by_observed_value() {
    let r = record(
        None,
        &obs(&[("ldarx", 5), ("dcbz", 0), ("host_invariant_breaks", 3)]),
    );
    assert_eq!(r["ldarx"].class, WitnessClass::AtLeast);
    assert_eq!(r["dcbz"].class, WitnessClass::Absent);
    assert_eq!(r["host_invariant_breaks"].class, WitnessClass::Exact);
}

/// Hand-promoting a class must survive re-recording, or the operator
/// would silently lose the decision on the next measurement.
#[test]
fn re_record_preserves_a_promoted_class() {
    let promoted = baseline(&[("ldarx", 5, WitnessClass::Exact)]);
    let r = record(Some(&promoted), &obs(&[("ldarx", 9)]));
    assert_eq!(r["ldarx"].value, 9);
    assert_eq!(r["ldarx"].class, WitnessClass::Exact);
}

/// `Absent` + nonzero would record a baseline that fails its own
/// check on every future run; it reclassifies instead.
#[test]
fn re_record_reclassifies_absent_that_started_firing() {
    let prev = baseline(&[("dcbz", 0, WitnessClass::Absent)]);
    let r = record(Some(&prev), &obs(&[("dcbz", 3)]));
    assert_eq!(r["dcbz"].value, 3);
    assert_eq!(r["dcbz"].class, WitnessClass::AtLeast);
    // Still-zero stays Absent.
    let r = record(Some(&prev), &obs(&[("dcbz", 0)]));
    assert_eq!(r["dcbz"].class, WitnessClass::Absent);
}

#[test]
fn parses_every_bench_line_shape() {
    let stderr = "\
BENCH_VRSAVE_WITNESS: mfvrsave_executed=4 vrsave_written=true
BENCH_HOST_INVARIANT_BREAKS: count=43
BENCH_ATOMIC_WITNESS: ldarx=1 stdcx=2 lwarx=3 stwcx=4
BENCH_MEM_FAULT_WITNESS: arm_entries=5 unmapped_routed=6
BENCH_RSX_LABEL_WRITES_WITNESS: count=7
BENCH_RSX_SET_REFERENCE_WITNESS: count=8
BENCH_DCBZ_WITNESS: count=9
BENCH_SPU_IMAGE_REGISTER_WITNESS: count=10
BENCH_SPU_THREAD_INIT_WITNESS: count=11
BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires=12 lwmutex_releases=13 cond_reacquires=14
BENCH_AUTHORITY_ID_WITNESS: program_authority_id=0x1010000001000003 lwmutex_unknown_locks=15
BENCH_PRX_LOAD_WITNESS: hle_stubs=16 not_found=17
";
    let w = parse_witness_lines(stderr).expect("all lines are well formed");
    assert_eq!(w.values["mfvrsave_executed"], 4);
    assert_eq!(w.values["vrsave_written"], 1, "booleans record as 0 or 1");
    assert_eq!(w.values["host_invariant_breaks"], 43);
    assert_eq!(w.values["stwcx"], 4);
    assert_eq!(w.values["mem_fault_unmapped_routed"], 6);
    assert_eq!(w.values["lwmutex_unknown_locks"], 15);
    assert_eq!(w.values["prx_load_hle_stubs"], 16);
    assert_eq!(w.values["prx_load_not_found"], 17);
    assert_eq!(
        w.values.len(),
        known_witness_names().len(),
        "every known witness parsed from the full line set"
    );
    assert_eq!(
        w.seen_lines.len(),
        12,
        "every emitted line is remembered for the presence check"
    );
    assert!(
        !w.values.contains_key("program_authority_id"),
        "ignored fields on a tracked line are skipped, not recorded"
    );
}

/// The old parser read a malformed value as zero, which is
/// indistinguishable from a boot that never reached the path.
#[test]
fn a_malformed_value_is_an_error_not_a_zero() {
    let errs = parse_witness_lines("BENCH_DCBZ_WITNESS: count=banana")
        .expect_err("a non-numeric count must not read as 0");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].to_string().contains("banana"));
}

#[test]
fn history_appends_nothing_when_nothing_moved() {
    let first =
        BootHistoryEntry::new_if_changed(None, 100, "ProcessExit", obs(&[("ldarx", 1)])).unwrap();
    assert!(first.changed.contains(&"steps".to_string()));
    assert!(
        BootHistoryEntry::new_if_changed(Some(&first), 100, "ProcessExit", obs(&[("ldarx", 1)]))
            .is_none(),
        "an identical run is not a move"
    );
}

#[test]
fn history_names_exactly_what_moved() {
    let first = BootHistoryEntry::new_if_changed(
        None,
        100,
        "ProcessExit",
        obs(&[("ldarx", 1), ("dcbz", 2)]),
    )
    .unwrap();
    let second = BootHistoryEntry::new_if_changed(
        Some(&first),
        101,
        "ProcessExit",
        obs(&[("ldarx", 1), ("dcbz", 5)]),
    )
    .expect("steps and dcbz moved");
    assert_eq!(
        second.changed,
        vec!["dcbz".to_string(), "steps".to_string()]
    );
}

#[test]
fn history_flags_a_witness_that_stopped_being_emitted() {
    let first =
        BootHistoryEntry::new_if_changed(None, 1, "ProcessExit", obs(&[("gone", 1)])).unwrap();
    let second = BootHistoryEntry::new_if_changed(Some(&first), 1, "ProcessExit", obs(&[]))
        .expect("losing a witness is a move");
    assert!(second
        .changed
        .iter()
        .any(|c| c.contains("no longer emitted")));
}

#[test]
fn history_lines_round_trip() {
    let e = BootHistoryEntry::new_if_changed(None, 7, "MaxSteps", obs(&[("a", 1)])).unwrap();
    let line = render_line(&e).unwrap();
    assert!(line.ends_with('\n'), "one entry per line");
    assert_eq!(parse(&line).unwrap(), vec![e]);
}

#[test]
fn history_parse_names_the_bad_line() {
    let good = render_line(
        &BootHistoryEntry::new_if_changed(None, 1, "ProcessExit", obs(&[("a", 1)])).unwrap(),
    )
    .unwrap();
    let err = parse(&format!("{good}not json\n")).expect_err("line 2 is not JSON");
    assert_eq!(err.line, 2);
    assert!(err.to_string().starts_with("line 2:"), "got {err}");
}
