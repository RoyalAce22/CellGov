//! Unit formatting and the phase-label table's edges.

use super::*;
#[cfg(debug_assertions)]
use crate::progress::serial::serial;

#[test]
fn bytes_format_in_binary_multiples() {
    let b = Unit::Bytes;
    assert_eq!(b.amount(0), "0 B");
    assert_eq!(b.amount(97), "97 B");
    assert_eq!(b.amount(1023), "1023 B");
    assert_eq!(b.amount(1024), "1 KiB");
    assert_eq!(b.amount(512 * 1024), "512 KiB");
    assert_eq!(b.amount((1 << 20) - 1), "1024 KiB");
    assert_eq!(b.amount(1 << 20), "1.0 MiB");
    assert_eq!(b.amount(38 * 1024 * 1024 + 200 * 1024), "38.2 MiB");
    assert_eq!(b.amount(1 << 30), "1.00 GiB");
    assert_eq!(b.amount(2 * 1024 * 1024 * 1024), "2.00 GiB");
    assert!(b.amount(u64::MAX).ends_with(" GiB"));
}

#[test]
fn counting_units_format_with_k_and_m_suffixes() {
    for unit in [Unit::Files, Unit::Steps, Unit::Cases, Unit::Items] {
        assert_eq!(unit.amount(0), "0");
        assert_eq!(unit.amount(999), "999");
        assert_eq!(unit.amount(1_000), "1.0k");
        assert_eq!(unit.amount(1_234), "1.2k");
        assert_eq!(unit.amount(999_999), "1000.0k");
        assert_eq!(unit.amount(1_000_000), "1.0M");
        assert_eq!(unit.amount(12_400_000), "12.4M");
        assert_eq!(unit.amount(100_000_000), "100.0M");
        assert_eq!(unit.amount(2_500_000_000), "2.50G");
    }
}

#[test]
fn rate_suffixes_name_the_unit_only_when_the_amount_does_not() {
    assert_eq!(Unit::Bytes.rate(38.2 * 1024.0 * 1024.0), "38.2 MiB/s");
    assert_eq!(Unit::Steps.rate(12_400_000.0), "12.4M steps/s");
    assert_eq!(Unit::Files.rate(3.0), "3 files/s");
    assert_eq!(Unit::Items.rate(1_500.0), "1.5k items/s");
    assert_eq!(Unit::Cases.rate(2_400.0), "2.4k cases/s");
    assert_eq!(Unit::Cases.tally(640), "640 cases");
    // The EWMA can drive a rate negative; the cast to u64 must not wrap.
    assert_eq!(Unit::Steps.rate(-5.0), "0 steps/s");
}

#[test]
fn the_eta_floor_is_a_kibibyte_for_bytes_and_one_for_counts() {
    assert_eq!(Unit::Bytes.eta_rate_floor(), 1024.0);
    for unit in [Unit::Files, Unit::Steps, Unit::Cases, Unit::Items] {
        assert_eq!(unit.eta_rate_floor(), 1.0);
    }
}

const T: Task = Task {
    verb: "Booting",
    tag: "boot",
    phases: &["loading", "stepping", "summarizing"],
    measured: 1,
    unit: Unit::Steps,
    items: "",
    streaming: false,
};

#[test]
fn every_code_the_table_covers_reads_its_own_label() {
    assert_eq!(T.phase_label(0), "loading");
    assert_eq!(T.phase_label(2), "summarizing");
}

#[test]
fn an_empty_table_answers_the_fallback_rather_than_treating_it_as_drift() {
    const EMPTY: Task = Task { phases: &[], ..T };
    assert_eq!(EMPTY.phase_label(0), FALLBACK_STATUS);
    assert_eq!(EMPTY.phase_label(u8::MAX), FALLBACK_STATUS);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "is past the 3 label(s)")]
fn a_phase_code_past_a_populated_table_names_the_drift() {
    // The panic reaches the bar's panic hook, which clears `LIVE_BAR`.
    let _s = serial();
    let _ = T.phase_label(3);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_phase_code_past_a_populated_table_falls_back_instead_of_panicking() {
    assert_eq!(T.phase_label(3), "loading");
    assert_eq!(T.phase_label(u8::MAX), "loading");
}

#[test]
fn the_status_budget_covers_the_empty_table_fallback_too() {
    const EMPTY: Task = Task { phases: &[], ..T };
    assert_eq!(EMPTY.max_status_len(), FALLBACK_STATUS.len());
    assert!(FALLBACK_STATUS.len() > DONE_STATUS.len());
}

#[test]
fn the_status_budget_covers_the_done_status_too() {
    // Every label here is shorter than "done".
    const SHORT: Task = Task {
        phases: &["a", "bc"],
        ..T
    };
    assert_eq!(SHORT.max_status_len(), DONE_STATUS.len());
    assert_eq!(T.max_status_len(), "summarizing".len());
}
