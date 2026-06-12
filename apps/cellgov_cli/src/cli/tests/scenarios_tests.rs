//! Named synthetic scenario execution, trace decode, and report determinism.

use super::*;
use cellgov_compare::observe_with_determinism_check;
use cellgov_explore::ExplorationConfig;
use cellgov_trace::TraceReader;

#[test]
fn every_named_scenario_runs_successfully() {
    for name in SCENARIOS {
        let (label, result) =
            run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
        assert_eq!(
            result.outcome,
            ScenarioOutcome::Stalled,
            "scenario {label} did not stall cleanly"
        );
        assert!(result.steps_taken > 0, "scenario {label} took zero steps");
    }
}

#[test]
fn unknown_scenario_returns_none() {
    assert!(run_scenario("nonexistent").is_none());
}

#[test]
fn report_is_deterministic_across_runs() {
    let (l1, r1) = run_scenario("fairness").unwrap();
    let (l2, r2) = run_scenario("fairness").unwrap();
    assert_eq!(report(l1, &r1), report(l2, &r2));
}

#[test]
fn dump_does_not_panic_for_any_scenario() {
    for name in SCENARIOS {
        let (_, result) = run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
        let records: Vec<_> = TraceReader::new(&result.trace_bytes)
            .enumerate()
            .map(|(i, r)| {
                r.unwrap_or_else(|e| {
                    panic!("scenario {name} decode failed at record index {i}: {e:?}")
                })
            })
            .collect();
        assert!(
            !records.is_empty(),
            "scenario {name} produced no trace records"
        );
    }
}

#[test]
fn report_includes_sync_hash_field() {
    let (label, result) = run_scenario("isa").unwrap();
    let r = report(label, &result);
    assert!(r.contains("sync_hash: 0x"));
}

#[test]
fn compare_succeeds_for_every_scenario() {
    for name in SCENARIOS {
        let factory = scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
        let result = observe_with_determinism_check(&factory, &[]);
        assert!(
            result.is_ok(),
            "compare failed for {name}: {:?}",
            result.err()
        );
    }
}

#[test]
fn scenario_factory_accepts_full_names() {
    assert!(scenario_factory("mailbox_send").is_some());
    assert!(scenario_factory("dma_block_unblock").is_some());
    assert!(scenario_factory("fake_isa").is_some());
    assert!(scenario_factory("round_robin_fairness").is_some());
    assert!(scenario_factory("write_conflict").is_some());
    assert!(scenario_factory("signal_update").is_some());
    assert!(scenario_factory("mailbox_roundtrip").is_some());
}

#[test]
fn scenario_factory_returns_none_for_unknown() {
    assert!(scenario_factory("nonexistent").is_none());
}

#[test]
fn explore_runs_for_multi_unit_scenarios() {
    for name in &["fairness", "conflict", "mailbox"] {
        let factory = scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
        let config = ExplorationConfig::default();
        let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
        assert!(
            result.is_some(),
            "scenario {name} should have branching points"
        );
    }
}

#[test]
fn explore_single_unit_returns_none() {
    let factory = scenario_factory("isa").expect("isa scenario exists");
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
    assert!(result.is_none(), "single-unit isa has no branching points");
}

#[test]
#[ignore] // ~7 min: runs 3 ELF microtests with full exploration
fn explore_micro_runs_for_elf_microtests() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let config = ExplorationConfig::default();
    for name in MICROTESTS {
        let base = repo_root.join(format!("tests/micro/{name}/build"));
        let ppu_path = base.join(format!("{name}.elf"));
        let spu_path = base.join("spu_main.elf");
        if !ppu_path.exists() || !spu_path.exists() {
            continue;
        }
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(repo_root).unwrap();
        let result = cellgov_explore::explore(|| build_lv2_fixture(name).build_runtime(), &config);
        std::env::set_current_dir(&prev).unwrap();
        assert!(
            result.is_some(),
            "microtest {name} should have branching points"
        );
        let r = result.unwrap();
        assert_eq!(
            r.outcome,
            cellgov_explore::OutcomeClass::ScheduleStable,
            "microtest {name} should be schedule-stable"
        );
    }
}
