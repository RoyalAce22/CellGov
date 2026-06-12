//! Observation extraction from synthetic scenario runs, including the cross-run determinism check.

use super::*;
use cellgov_testkit::fixtures;
use cellgov_testkit::runner::run;

#[test]
fn observe_maps_stalled_to_completed() {
    let result = run(fixtures::round_robin_fairness_scenario(2, 3));
    let obs = observe(&result, &[]);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
}

#[test]
fn observe_carries_state_hashes() {
    let result = run(fixtures::round_robin_fairness_scenario(2, 3));
    let obs = observe(&result, &[]);
    let hashes = obs.state_hashes.unwrap();
    assert_eq!(hashes.memory, result.final_memory_hash);
    assert_eq!(hashes.unit_status, result.final_unit_status_hash);
    assert_eq!(hashes.sync, result.final_sync_hash);
}

#[test]
fn observe_metadata_says_cellgov() {
    let result = run(fixtures::round_robin_fairness_scenario(1, 1));
    let obs = observe(&result, &[]);
    assert_eq!(obs.metadata.runner, "cellgov");
    assert!(obs.metadata.steps.is_some());
}

#[test]
fn observe_extracts_events_from_mailbox_scenario() {
    let result = run(fixtures::mailbox_send_scenario(3));
    let obs = observe(&result, &[]);
    assert!(
        obs.events
            .iter()
            .any(|e| e.kind == ObservedEventKind::MailboxSend),
        "expected at least one MailboxSend event"
    );
}

#[test]
fn observe_extracts_block_wake_from_dma_scenario() {
    let result = run(fixtures::dma_block_unblock_scenario());
    let obs = observe(&result, &[]);
    assert!(
        obs.events
            .iter()
            .any(|e| e.kind == ObservedEventKind::UnitBlock),
        "expected at least one UnitBlock event"
    );
    assert!(
        obs.events
            .iter()
            .any(|e| e.kind == ObservedEventKind::DmaComplete),
        "expected at least one DmaComplete event"
    );
}

#[test]
fn observe_event_sequences_are_monotonic() {
    let result = run(fixtures::mailbox_roundtrip_scenario(0x42));
    let obs = observe(&result, &[]);
    for (i, event) in obs.events.iter().enumerate() {
        assert_eq!(event.sequence, i as u32);
    }
}

#[test]
fn two_identical_runs_produce_identical_observations() {
    let r1 = run(fixtures::fake_isa_scenario());
    let r2 = run(fixtures::fake_isa_scenario());
    let regions = vec![RegionDescriptor {
        name: "shared".into(),
        addr: 0,
        size: 8,
    }];
    let o1 = observe(&r1, &regions);
    let o2 = observe(&r2, &regions);
    assert_eq!(o1.outcome, o2.outcome);
    assert_eq!(o1.memory_regions, o2.memory_regions);
    assert_eq!(o1.events, o2.events);
    assert_eq!(o1.state_hashes, o2.state_hashes);
}

type ScenarioFactory = Box<dyn Fn() -> cellgov_testkit::ScenarioFixture>;

#[test]
fn determinism_check_passes_for_all_scenarios() {
    let scenarios: Vec<(&str, ScenarioFactory)> = vec![
        (
            "fairness",
            Box::new(|| fixtures::round_robin_fairness_scenario(3, 5)),
        ),
        (
            "conflict",
            Box::new(|| fixtures::write_conflict_scenario(3)),
        ),
        (
            "mailbox",
            Box::new(|| fixtures::mailbox_roundtrip_scenario(0x42)),
        ),
        ("dma", Box::new(fixtures::dma_block_unblock_scenario)),
        ("send", Box::new(|| fixtures::mailbox_send_scenario(5))),
        ("signal", Box::new(|| fixtures::signal_update_scenario(4))),
        ("isa", Box::new(fixtures::fake_isa_scenario)),
    ];
    for (name, factory) in &scenarios {
        let result = observe_with_determinism_check(factory, &[]);
        assert!(
            result.is_ok(),
            "determinism check failed for {name}: {:?}",
            result.err()
        );
    }
}
