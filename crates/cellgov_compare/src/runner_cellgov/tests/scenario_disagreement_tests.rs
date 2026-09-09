//! A refusal only one run raised is a disagreement; one both runs
//! raised the same way is a failed run.

use std::cell::Cell;

use super::*;
use cellgov_core::AddressSpaceId;
use cellgov_testkit::fixtures;

/// A factory whose first call builds `first` and whose second builds
/// `second`; the check calls it exactly twice.
fn alternating(
    first: fn() -> ScenarioFixture,
    second: fn() -> ScenarioFixture,
) -> impl Fn() -> ScenarioFixture {
    let calls = Cell::new(0u32);
    move || {
        let n = calls.get();
        calls.set(n + 1);
        if n == 0 {
            first()
        } else {
            second()
        }
    }
}

/// 256 bytes of memory, so `window` and `high` read.
fn wide() -> ScenarioFixture {
    fixtures::dma_block_unblock_scenario()
}

/// 16 bytes of memory, so `window` and `high` are unmapped.
fn narrow() -> ScenarioFixture {
    fixtures::round_robin_fairness_scenario(2, 3)
}

fn region(name: &str, addr: u64) -> RegionDescriptor {
    RegionDescriptor {
        name: name.into(),
        space: AddressSpaceId::BOOT,
        addr,
        size: 8,
    }
}

fn window() -> RegionDescriptor {
    region("window", 0x80)
}

fn unreadable(err: &ObserveError, name: &str) -> bool {
    matches!(err, ObserveError::Region(RegionExtractError::Unreadable { name: n, .. }) if n == name)
}

#[test]
fn a_region_only_the_second_run_refuses_is_a_disagreement_naming_the_second_run() {
    let err = observe_with_determinism_check(alternating(wide, narrow), &[window()])
        .expect_err("the second run maps 16 bytes and cannot read 0x80");
    let DeterminismError::ObserveDisagreement(d) = &err else {
        panic!("expected a disagreement, got {err:?}");
    };
    let ObserveDisagreement::SecondOnly(why) = d.as_ref() else {
        panic!("expected a second-run-only refusal, got {err:?}");
    };
    assert!(unreadable(why, "window"), "{why:?}");
    let text = err.to_string();
    assert!(
        text.starts_with("the second run produced no observation"),
        "{text}"
    );
    assert!(text.contains("region window "), "{text}");
}

#[test]
fn a_region_only_the_first_run_refuses_is_a_disagreement_naming_the_first_run() {
    let err = observe_with_determinism_check(alternating(narrow, wide), &[window()])
        .expect_err("the first run maps 16 bytes and cannot read 0x80");
    let DeterminismError::ObserveDisagreement(d) = &err else {
        panic!("expected a disagreement, got {err:?}");
    };
    let ObserveDisagreement::FirstOnly(why) = d.as_ref() else {
        panic!("expected a first-run-only refusal, got {err:?}");
    };
    assert!(unreadable(why, "window"), "{why:?}");
    assert!(
        err.to_string()
            .starts_with("the first run produced no observation"),
        "{err}"
    );
}

#[test]
fn a_region_both_runs_refuse_the_same_way_is_a_failed_run_not_a_disagreement() {
    let err =
        observe_with_determinism_check(narrow, &[window()]).expect_err("neither run maps 0x80");
    let DeterminismError::Observe(why) = &err else {
        panic!("expected a plain refusal, got {err:?}");
    };
    assert!(unreadable(why, "window"), "{why:?}");
}

#[test]
fn two_runs_refusing_for_different_reasons_is_a_disagreement_carrying_both() {
    // The narrow run refuses `low` first; the wide run reads `low` and
    // refuses `high`, so the two refusals name different regions.
    let regions = [region("low", 0x10), region("high", 0x100)];
    let err = observe_with_determinism_check(alternating(narrow, wide), &regions)
        .expect_err("each run refuses a different region");
    let DeterminismError::ObserveDisagreement(d) = &err else {
        panic!("expected a disagreement, got {err:?}");
    };
    let ObserveDisagreement::Both { first, second } = d.as_ref() else {
        panic!("expected both runs to refuse differently, got {err:?}");
    };
    assert!(unreadable(first, "low"), "{first:?}");
    assert!(unreadable(second, "high"), "{second:?}");
    let text = err.to_string();
    assert!(
        text.contains("region low ") && text.contains("region high "),
        "{text}"
    );
}

#[test]
fn a_region_both_runs_read_still_compares_the_observations() {
    let obs = observe_with_determinism_check(wide, &[window()]).expect("both runs read 0x80");
    assert_eq!(obs.memory_regions.len(), 1);
    assert_eq!(obs.memory_regions[0].name, "window");
}
