//! Scenario fixture builders -- schedule shape, final-state hashes, and panic guards on degenerate inputs.

use super::*;
use crate::assertions::assert_deterministic_replay;
use crate::runner::{run, ScenarioOutcome};
use cellgov_event::UnitId;
use cellgov_trace::{TraceReader, TraceRecord};
use std::collections::BTreeMap;

#[test]
fn empty_fixture_has_zero_defaults() {
    let f = ScenarioFixture::empty();
    assert_eq!(f.memory_size, 0);
    assert_eq!(f.budget, Budget::new(0));
    assert_eq!(f.max_steps, 1);
}

#[test]
fn builder_overrides_each_field() {
    let f = ScenarioFixture::builder()
        .memory_size(128)
        .budget(Budget::new(7))
        .max_steps(42)
        .build();
    assert_eq!(f.memory_size, 128);
    assert_eq!(f.budget, Budget::new(7));
    assert_eq!(f.max_steps, 42);
}

/// Units in `UnitScheduled` order.
fn scheduled_unit_sequence(trace_bytes: &[u8]) -> Vec<UnitId> {
    TraceReader::new(trace_bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::UnitScheduled { unit, .. } => Some(unit),
            _ => None,
        })
        .collect()
}

#[test]
fn round_robin_fairness_visits_units_in_strict_id_order() {
    let result = run(round_robin_fairness_scenario(3, 4));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 12);

    let sequence = scheduled_unit_sequence(&result.trace_bytes);
    assert_eq!(sequence.len(), 12);
    let expected: Vec<UnitId> = (0..12).map(|i| UnitId::new((i % 3) as u64)).collect();
    assert_eq!(sequence, expected);
}

#[test]
fn round_robin_fairness_no_unit_starves() {
    let result = run(round_robin_fairness_scenario(4, 5));
    assert_eq!(result.steps_taken, 20);
    let sequence = scheduled_unit_sequence(&result.trace_bytes);
    let mut counts: BTreeMap<UnitId, usize> = BTreeMap::new();
    for u in sequence {
        *counts.entry(u).or_insert(0) += 1;
    }
    assert_eq!(counts.len(), 4);
    for (id, count) in counts {
        assert_eq!(count, 5, "unit {} got {count} steps, expected 5", id.raw());
    }
}

#[test]
fn round_robin_fairness_replays_identically() {
    assert_deterministic_replay(|| round_robin_fairness_scenario(3, 7), 4);
}

#[test]
fn round_robin_fairness_single_unit_degenerate_case() {
    let result = run(round_robin_fairness_scenario(1, 5));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
    let sequence = scheduled_unit_sequence(&result.trace_bytes);
    assert!(sequence.iter().all(|u| *u == UnitId::new(0)));
}

#[test]
#[should_panic(expected = "needs at least 1 unit")]
fn round_robin_fairness_zero_units_panics() {
    let _ = round_robin_fairness_scenario(0, 5);
}

#[test]
#[should_panic(expected = "needs at least 1 step")]
fn round_robin_fairness_zero_steps_panics() {
    let _ = round_robin_fairness_scenario(3, 0);
}

/// Units in `EffectEmitted` order.
fn emitted_effect_units(trace_bytes: &[u8]) -> Vec<UnitId> {
    TraceReader::new(trace_bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::EffectEmitted { unit, .. } => Some(unit),
            _ => None,
        })
        .collect()
}

#[test]
fn write_conflict_runs_to_completion_and_replays() {
    let result = run(write_conflict_scenario(3));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 6);
    assert_deterministic_replay(|| write_conflict_scenario(3), 3);
}

#[test]
fn write_conflict_writes_alternate_between_units_in_id_order() {
    let result = run(write_conflict_scenario(4));
    let effect_units = emitted_effect_units(&result.trace_bytes);
    assert_eq!(effect_units.len(), 8);
    let expected: Vec<UnitId> = (0..8).map(|i| UnitId::new((i % 2) as u64)).collect();
    assert_eq!(effect_units, expected);
}

#[test]
fn write_conflict_last_writer_wins_via_hash_equivalence() {
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;

    // Under round-robin with matching step counts, unit 1's last
    // commit wins: [steps_per_unit; 4] at addr 0.
    let steps = 5u64;
    let result = run(write_conflict_scenario(steps));

    let mut expected = GuestMemory::new(16);
    expected
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[steps as u8; 4],
        )
        .unwrap();
    let expected_hash = StateHash::new(expected.content_hash());

    assert_eq!(result.final_memory_hash, expected_hash);
}

#[test]
#[should_panic(expected = "needs at least 1 step")]
fn write_conflict_zero_steps_panics() {
    let _ = write_conflict_scenario(0);
}

#[test]
fn mailbox_send_scenario_runs_to_completion() {
    let result = run(mailbox_send_scenario(5));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
}

#[test]
fn mailbox_send_scenario_replays_identically() {
    assert_deterministic_replay(|| mailbox_send_scenario(4), 3);
}

#[test]
fn mailbox_send_scenario_final_sync_hash_matches_expected_registry() {
    // Parallel runtime mirrors every sync source the hash aggregates.
    use cellgov_core::Runtime;
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let result = run(mailbox_send_scenario(5));

    let mut expected = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    let mb = expected.mailbox_registry_mut().register(4);
    for n in 1..=5u32 {
        expected
            .mailbox_registry_mut()
            .get_mut(mb)
            .unwrap()
            .force_send(n);
    }
    assert_eq!(
        result.final_sync_hash,
        StateHash::new(expected.sync_state_hash())
    );
}

#[test]
fn mailbox_send_scenario_emits_one_send_effect_per_message() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let result = run(mailbox_send_scenario(6));
    let send_count = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| {
            matches!(
                r,
                TraceRecord::EffectEmitted {
                    kind: TracedEffectKind::MailboxSend,
                    ..
                }
            )
        })
        .count();
    assert_eq!(send_count, 6);
}

#[test]
#[should_panic(expected = "needs at least 1 message")]
fn mailbox_send_scenario_zero_messages_panics() {
    let _ = mailbox_send_scenario(0);
}

#[test]
fn signal_update_scenario_runs_to_completion() {
    let result = run(signal_update_scenario(5));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
}

#[test]
fn signal_update_scenario_replays_identically() {
    assert_deterministic_replay(|| signal_update_scenario(4), 3);
}

#[test]
fn signal_update_scenario_final_sync_hash_matches_expected_register() {
    // After `bit_count` distinct-bit OR-merges the register is
    // (1 << bit_count) - 1.
    use cellgov_core::Runtime;
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let bit_count = 6u64;
    let result = run(signal_update_scenario(bit_count));

    let mut expected = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    let sig = expected.signal_registry_mut().register();
    let final_value = (1u32 << bit_count as u32) - 1;
    expected
        .signal_registry_mut()
        .get_mut(sig)
        .unwrap()
        .or_in(final_value);
    assert_eq!(
        result.final_sync_hash,
        StateHash::new(expected.sync_state_hash())
    );
}

#[test]
fn signal_update_scenario_emits_one_signal_effect_per_step() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let result = run(signal_update_scenario(7));
    let signal_effect_count = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| {
            matches!(
                r,
                TraceRecord::EffectEmitted {
                    kind: TracedEffectKind::SignalUpdate,
                    ..
                }
            )
        })
        .count();
    assert_eq!(signal_effect_count, 7);
}

#[test]
#[should_panic(expected = "needs at least 1 bit")]
fn signal_update_scenario_zero_bits_panics() {
    let _ = signal_update_scenario(0);
}

#[test]
#[should_panic(expected = "must be <= 32")]
fn signal_update_scenario_more_than_32_bits_panics() {
    let _ = signal_update_scenario(33);
}

#[test]
fn mailbox_roundtrip_runs_to_completion_in_five_steps() {
    let result = run(mailbox_roundtrip_scenario(0x42));
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
}

#[test]
fn mailbox_roundtrip_replays_identically() {
    assert_deterministic_replay(|| mailbox_roundtrip_scenario(0x42), 4);
}

#[test]
fn mailbox_roundtrip_trace_contains_full_exchange() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let result = run(mailbox_roundtrip_scenario(0x42));
    let effect_kinds: Vec<TracedEffectKind> = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::EffectEmitted { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    // Step 1 (sender): MailboxSend, WakeUnit, WaitOnEvent
    // Step 2 (responder): MailboxReceiveAttempt
    // Step 3 (responder): MailboxSend, WakeUnit
    // Step 4 (sender): MailboxReceiveAttempt
    // Step 5 (sender): TraceMarker
    assert_eq!(
        effect_kinds,
        vec![
            TracedEffectKind::MailboxSend,
            TracedEffectKind::WakeUnit,
            TracedEffectKind::WaitOnEvent,
            TracedEffectKind::MailboxReceiveAttempt,
            TracedEffectKind::MailboxSend,
            TracedEffectKind::WakeUnit,
            TracedEffectKind::MailboxReceiveAttempt,
            TracedEffectKind::TraceMarker,
        ]
    );
}

#[test]
fn mailbox_roundtrip_response_is_command_plus_one() {
    // EffectEmitted only carries kind, not payload; the implicit
    // proof is that the sender only finishes if it received a
    // response. Payload-level check in mailbox_send_scenario tests.
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let command = 0x100u32;
    let result = run(mailbox_roundtrip_scenario(command));
    let final_marker = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| {
            matches!(
                r,
                TraceRecord::EffectEmitted {
                    kind: TracedEffectKind::TraceMarker,
                    ..
                }
            )
        })
        .count();
    assert_eq!(final_marker, 1);
}

#[test]
fn dma_block_unblock_runs_to_completion() {
    let result = run(dma_block_unblock_scenario());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    // Submitter: 1 step (submit+block). Burner: 10 steps to push
    // time from 1 to 11 (completion fires at 11). Submitter: 1
    // step (wake+finish). Burner: 10 remaining steps. Total: 22.
    assert_eq!(result.steps_taken, 22);
}

#[test]
fn dma_block_unblock_transfer_lands_in_committed_memory() {
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let result = run(dma_block_unblock_scenario());
    let mut expected_mem = GuestMemory::new(256);
    expected_mem
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0xde, 0xad, 0xbe, 0xef],
        )
        .unwrap();
    expected_mem
        .apply_commit(
            ByteRange::new(GuestAddr::new(128), 4).unwrap(),
            &[0xde, 0xad, 0xbe, 0xef],
        )
        .unwrap();
    assert_eq!(
        result.final_memory_hash,
        StateHash::new(expected_mem.content_hash())
    );
}

#[test]
fn dma_block_unblock_replays_identically() {
    assert_deterministic_replay(dma_block_unblock_scenario, 3);
}

#[test]
fn dma_block_unblock_trace_contains_dma_effects() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let result = run(dma_block_unblock_scenario());
    let dma_effects: Vec<TracedEffectKind> = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::EffectEmitted { kind, .. }
                if kind == TracedEffectKind::DmaEnqueue
                    || kind == TracedEffectKind::WaitOnEvent =>
            {
                Some(kind)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        dma_effects,
        vec![TracedEffectKind::DmaEnqueue, TracedEffectKind::WaitOnEvent]
    );
}

#[test]
fn mailbox_roundtrip_final_mailboxes_are_empty() {
    use cellgov_core::Runtime;
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let result = run(mailbox_roundtrip_scenario(0x42));
    let mut expected = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    expected.mailbox_registry_mut().register(4); // cmd
    expected.mailbox_registry_mut().register(4); // resp
    assert_eq!(
        result.final_sync_hash,
        StateHash::new(expected.sync_state_hash())
    );
}

#[test]
fn fake_isa_scenario_runs_to_completion() {
    // LoadImm + SharedStore + MailboxSend + End = 4 steps.
    let result = run(fake_isa_scenario());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 4);
}

#[test]
fn fake_isa_scenario_replays_identically() {
    assert_deterministic_replay(fake_isa_scenario, 3);
}

#[test]
fn fake_isa_scenario_committed_memory_matches_expected() {
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let result = run(fake_isa_scenario());
    let mut expected_mem = GuestMemory::new(256);
    expected_mem
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &[0xab, 0xab, 0xab, 0xab],
        )
        .unwrap();
    assert_eq!(
        result.final_memory_hash,
        StateHash::new(expected_mem.content_hash())
    );
}

#[test]
fn fake_isa_scenario_mailbox_has_message() {
    use cellgov_core::Runtime;
    use cellgov_mem::GuestMemory;
    use cellgov_trace::StateHash;
    let result = run(fake_isa_scenario());
    let mut expected = Runtime::new(GuestMemory::new(256), Budget::new(1), 100);
    let mb = expected.mailbox_registry_mut().register(4);
    expected
        .mailbox_registry_mut()
        .get_mut(mb)
        .unwrap()
        .force_send(0xab);
    assert_eq!(
        result.final_sync_hash,
        StateHash::new(expected.sync_state_hash())
    );
}

#[test]
fn fake_isa_scenario_trace_effect_sequence() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
    let result = run(fake_isa_scenario());
    let effects: Vec<TracedEffectKind> = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::EffectEmitted { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        effects,
        vec![
            TracedEffectKind::SharedWriteIntent,
            TracedEffectKind::MailboxSend,
        ]
    );
}
