//! Observation extraction from synthetic scenario runs, including the cross-run determinism check.

use super::*;
use cellgov_testkit::fixtures;
use cellgov_testkit::runner::run;

#[test]
fn a_checked_run_carries_the_first_invariant_break_beside_the_observation() {
    use cellgov_core::Runtime;

    let clean = observe_checked(|| fixtures::round_robin_fairness_scenario(2, 3), &[])
        .expect("both runs observe");
    assert_eq!(
        clean.first_invariant_break, None,
        "a run that broke no invariant gives its driver no line to report"
    );

    let broken = observe_checked(
        || {
            cellgov_testkit::fixtures::ScenarioFixture::builder()
                .register(|rt: &mut Runtime| {
                    rt.lv2_host_mut()
                        .log_invariant_break("test.site", format_args!("details here"));
                })
                .build()
        },
        &[],
    )
    .expect("both runs observe");
    assert_eq!(
        broken.first_invariant_break.as_deref(),
        Some("lv2 host invariant break at test.site: details here (the first of 1)"),
        "the diagnostic reaches the driver beside the observation, never inside it"
    );
    assert_eq!(
        broken.observation.metadata.runner, "cellgov",
        "the observation the comparison reads is unchanged by the diagnostic"
    );
}

#[test]
fn observe_maps_stalled_to_completed() {
    let result = run(fixtures::round_robin_fairness_scenario(2, 3));
    let obs = observe(&result, &[]).expect("trace decodes");
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
}

#[test]
fn observe_carries_state_hashes() {
    let result = run(fixtures::round_robin_fairness_scenario(2, 3));
    let obs = observe(&result, &[]).expect("trace decodes");
    let hashes = obs.state_hashes.unwrap();
    assert_eq!(hashes.memory, result.final_memory_hash);
    assert_eq!(hashes.unit_status, result.final_unit_status_hash);
    assert_eq!(hashes.sync, result.final_sync_hash);
}

#[test]
fn observe_metadata_says_cellgov() {
    let result = run(fixtures::round_robin_fairness_scenario(1, 1));
    let obs = observe(&result, &[]).expect("trace decodes");
    assert_eq!(obs.metadata.runner, "cellgov");
    assert!(obs.metadata.steps.is_some());
}

#[test]
fn observe_extracts_events_from_mailbox_scenario() {
    let result = run(fixtures::mailbox_send_scenario(3));
    let obs = observe(&result, &[]).expect("trace decodes");
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
    let obs = observe(&result, &[]).expect("trace decodes");
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
    let obs = observe(&result, &[]).expect("trace decodes");
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
        space: cellgov_core::AddressSpaceId::BOOT,
        addr: 0,
        size: 8,
    }];
    let o1 = observe(&r1, &regions).expect("trace decodes");
    let o2 = observe(&r2, &regions).expect("trace decodes");
    assert_eq!(o1.outcome, o2.outcome);
    assert_eq!(o1.memory_regions, o2.memory_regions);
    assert_eq!(o1.events, o2.events);
    assert_eq!(o1.state_hashes, o2.state_hashes);
}

/// Byte offset of the `n`th record in `trace`, walking the same
/// decoder the observer uses.
fn record_offset(trace: &[u8], n: usize) -> usize {
    let mut reader = cellgov_trace::TraceReader::new(trace);
    for _ in 0..n {
        reader
            .next()
            .expect("record exists")
            .expect("record decodes");
    }
    reader.position()
}

#[test]
fn a_corrupted_record_fails_observation_naming_its_index_and_offset() {
    let mut result = run(fixtures::mailbox_roundtrip_scenario(0x42));
    let clean = observe(&result, &[]).expect("the unmodified trace decodes");
    assert!(
        clean.events.len() > 2,
        "the fixture must yield events past the corruption point for the prefix to matter"
    );

    let offset = record_offset(&result.trace_bytes, 2);
    result.trace_bytes[offset] = 0xff;

    let err = observe(&result, &[]).expect_err("a bad tag must not be flattened past");
    assert_eq!(
        err,
        ObserveError::TraceDecode(TraceDecodeError {
            index: 2,
            offset,
            source: cellgov_trace::DecodeError::UnknownTag(0xff),
        })
    );
}

#[test]
fn a_truncated_trace_fails_observation_at_the_cut_record() {
    let mut result = run(fixtures::mailbox_roundtrip_scenario(0x42));
    let offset = record_offset(&result.trace_bytes, 3);
    // Keep one byte of record 3 so the stream ends mid-record rather
    // than on a boundary, which would read as a clean end.
    result.trace_bytes.truncate(offset + 1);

    let err = observe(&result, &[]).expect_err("a mid-record end must not read as a clean end");
    let ObserveError::TraceDecode(err) = err else {
        panic!("expected a trace decode failure, got {err:?}");
    };
    assert_eq!(err.index, 3);
    assert_eq!(err.offset, offset);
    assert_eq!(err.source, cellgov_trace::DecodeError::Truncated);
}

#[test]
fn the_determinism_check_reports_a_decode_failure_before_comparing() {
    // A factory whose runtime writes a trace record the decoder rejects
    // is not constructible from the public fixtures, so the propagation
    // is pinned on the error type instead: a TraceDecodeError converts
    // into the check's own error and displays its position.
    let err = DeterminismError::from(ObserveError::from(TraceDecodeError {
        index: 4,
        offset: 0x80,
        source: cellgov_trace::DecodeError::Truncated,
    }));
    assert!(matches!(
        err,
        DeterminismError::Observe(ObserveError::TraceDecode(_))
    ));
    let text = err.to_string();
    assert!(text.contains("record 4"), "{text}");
    assert!(text.contains("offset 128"), "{text}");
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
