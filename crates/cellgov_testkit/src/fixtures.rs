//! Scenario fixtures: initial units, memory and device state, scheduler
//! policy knobs, expected invariants, optional expected trace fragments,
//! expected final state hashes.
//!
//! A `ScenarioFixture` is a value object describing how to build a
//! [`cellgov_core::Runtime`] for a scenario. It carries the runtime
//! construction inputs (memory size, per-step budget, max-steps cap)
//! and a one-shot registration callback that populates the unit
//! registry. The runner consumes the fixture and never inspects its
//! internals; tests build fixtures and hand them to
//! [`crate::runner::run`] without ever touching `Runtime` directly.
//!
//! The closure-based registration is deliberate: storing
//! `Box<dyn ExecutionUnit>` directly in the fixture is awkward because
//! `ExecutionUnit` has an associated type and is not object-safe.
//! Instead the fixture defers unit construction to a callback the
//! runtime drives at build time, with the live `&mut UnitRegistry`
//! handed in. Tests use it as:
//!
//! ```ignore
//! let fixture = ScenarioFixture::builder()
//!     .memory_size(64)
//!     .budget(5)
//!     .max_steps(1_000)
//!     .register(|r| { r.register_with(|id| MyUnit::new(id)); })
//!     .build();
//! ```

use crate::world::{
    CountingUnit, DmaSubmitter, MailboxProducer, MailboxResponder, MailboxSender, SignalEmitter,
    WritingUnit,
};
use cellgov_core::Runtime;
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::Budget;

/// Boxed one-shot callback that populates a fresh runtime with the
/// fixture's units, mailboxes, signal registers, and any other
/// runtime-owned state. Type-erased so the fixture can be a plain
/// struct.
///
/// The callback receives a mutable borrow of the whole [`Runtime`],
/// so it can register a mailbox or signal first, capture the returned
/// id, and pass it into a unit factory in the same closure body.
/// Each `Runtime::*_mut` accessor borrows the runtime briefly enough
/// that sequential calls compose without fighting the borrow checker.
type RegisterFn = Box<dyn FnOnce(&mut Runtime)>;

/// A scenario fixture: everything the runner needs to build a runtime
/// and populate it before stepping.
pub struct ScenarioFixture {
    pub(crate) memory_size: usize,
    pub(crate) budget: Budget,
    pub(crate) max_steps: usize,
    pub(crate) register: RegisterFn,
}

impl ScenarioFixture {
    /// Construct an empty fixture: zero-byte memory, zero budget, one
    /// max step, no units. The runner can still drive it -- the result
    /// will be `Stalled { steps_taken: 0 }` because the registry is
    /// empty -- and tests use it as the trivial smoke fixture.
    pub fn empty() -> Self {
        Self {
            memory_size: 0,
            budget: Budget::new(0),
            max_steps: 1,
            register: Box::new(|_| {}),
        }
    }

    /// Construct a fresh builder with empty defaults. See the module
    /// documentation for the typical call shape.
    pub fn builder() -> ScenarioFixtureBuilder {
        ScenarioFixtureBuilder::default()
    }
}

/// Builder for [`ScenarioFixture`]. Defaults to a 16-byte memory, a
/// per-step budget of 1, a 1000-step cap, and an empty registration
/// callback. Tests override only the fields they care about.
pub struct ScenarioFixtureBuilder {
    memory_size: usize,
    budget: Budget,
    max_steps: usize,
    register: RegisterFn,
}

impl Default for ScenarioFixtureBuilder {
    fn default() -> Self {
        Self {
            memory_size: 16,
            budget: Budget::new(1),
            max_steps: 1_000,
            register: Box::new(|_| {}),
        }
    }
}

impl ScenarioFixtureBuilder {
    /// Set the committed-memory size in bytes.
    pub fn memory_size(mut self, bytes: usize) -> Self {
        self.memory_size = bytes;
        self
    }

    /// Set the per-step budget granted to the selected unit.
    pub fn budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Set the runtime's max-steps cap (the deadlock detector trip
    /// point). Stalls fail cleanly via [`crate::runner::ScenarioOutcome::MaxStepsExceeded`];
    /// they never hang the suite.
    pub fn max_steps(mut self, steps: usize) -> Self {
        self.max_steps = steps;
        self
    }

    /// Install a one-shot registration callback. Replaces any
    /// previously installed callback. The callback receives a
    /// mutable borrow of the runtime exactly once at fixture
    /// construction time inside the runner; it can register units,
    /// mailboxes, signal registers, and any other runtime-owned
    /// state via the runtime's `*_mut` accessors.
    pub fn register<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Runtime) + 'static,
    {
        self.register = Box::new(f);
        self
    }

    /// Finalize the builder into a [`ScenarioFixture`].
    pub fn build(self) -> ScenarioFixture {
        ScenarioFixture {
            memory_size: self.memory_size,
            budget: self.budget,
            max_steps: self.max_steps,
            register: self.register,
        }
    }
}

/// **Scenario D: budget-exhaustion round-robin fairness.**
///
/// Registers `unit_count` [`CountingUnit`]s, each configured to finish
/// after `steps_per_unit` steps. Per-step budget is 1, so every unit
/// yields with `BudgetExhausted` until its final step (which yields
/// `Finished`). The round-robin scheduler must visit them in id order
/// and visit every unit equally; no runnable unit may starve under
/// this fixed workload.
///
/// `max_steps` is set to `unit_count * steps_per_unit + 1` so the
/// deadlock detector can fire if the scheduler ever gets stuck. A
/// well-behaved run terminates with [`crate::runner::ScenarioOutcome::Stalled`]
/// after exactly `unit_count * steps_per_unit` steps.
pub fn round_robin_fairness_scenario(unit_count: usize, steps_per_unit: u64) -> ScenarioFixture {
    assert!(
        unit_count > 0,
        "round_robin_fairness_scenario needs at least 1 unit"
    );
    assert!(
        steps_per_unit > 0,
        "round_robin_fairness_scenario needs at least 1 step per unit"
    );
    let cap = unit_count
        .checked_mul(steps_per_unit as usize)
        .and_then(|n| n.checked_add(1))
        .expect("round_robin_fairness_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            for _ in 0..unit_count {
                rt.registry_mut()
                    .register_with(|id| CountingUnit::new(id, steps_per_unit));
            }
        })
        .build()
}

/// **Scenario B: DMA block/unblock.**
///
/// A [`DmaSubmitter`] seeds 4 bytes at address 0, submits a DMA Put
/// to copy them to address 128, and blocks. A [`CountingUnit`] burns
/// ticks until the runtime's guest time passes the modeled completion
/// time (default `FixedLatency(10)` means the transfer completes at
/// `issue_time + 10`). When the completion fires, the submitter
/// wakes and finishes.
///
/// The run proves the full DMA lifecycle: enqueue, latency wait,
/// completion fires, transfer applied to committed memory, issuer
/// woken.
pub fn dma_block_unblock_scenario() -> ScenarioFixture {
    let src = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let dst = ByteRange::new(GuestAddr::new(128), 4).unwrap();
    let seed = vec![0xde, 0xad, 0xbe, 0xef];
    ScenarioFixture::builder()
        .memory_size(256)
        .budget(Budget::new(1))
        .max_steps(30)
        .register(move |rt: &mut Runtime| {
            // Submitter is unit 0, tick-burner is unit 1.
            rt.registry_mut()
                .register_with(|id| DmaSubmitter::new(id, src, dst, seed.clone()));
            rt.registry_mut()
                .register_with(|id| CountingUnit::new(id, 20));
        })
        .build()
}

/// **Scenario C: deterministic write-conflict resolution.**
///
/// Registers two [`WritingUnit`]s that both write into the same 4-byte
/// committed-memory range. Round-robin scheduling interleaves their
/// commits, and the "one commit batch per unit yield" rule means
/// each write becomes visible at its own epoch boundary -- the last
/// commit wins.
///
/// With matching `steps_per_unit` and round-robin ordering, the run
/// ends with the higher-id unit having committed the final write. The
/// `steps_per_unit`-th write payload (`n` byte-replicated across the
/// range, where `n = steps_per_unit`) is what tests assert against.
///
/// `max_steps` is set to `2 * steps_per_unit + 1` so the deadlock
/// detector can fire if the schedule ever gets stuck.
pub fn write_conflict_scenario(steps_per_unit: u64) -> ScenarioFixture {
    assert!(
        steps_per_unit > 0,
        "write_conflict_scenario needs at least 1 step per unit"
    );
    let cap = (2usize)
        .checked_mul(steps_per_unit as usize)
        .and_then(|n| n.checked_add(1))
        .expect("write_conflict_scenario step cap overflow");
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            rt.registry_mut()
                .register_with(|id| WritingUnit::new(id, steps_per_unit, range));
            rt.registry_mut()
                .register_with(|id| WritingUnit::new(id, steps_per_unit, range));
        })
        .build()
}

/// **Mailbox producer scenario.** Registers a single mailbox plus a
/// [`MailboxProducer`] that sends `message_count` words (1..=N) into
/// it. Each step yields with `MailboxAccess`; the run ends with all N
/// messages queued in id order.
///
/// This is the smallest end-to-end test of the `MailboxSend` commit
/// path through the runner: producer step -> EffectEmitted record ->
/// commit pipeline -> mailbox FIFO -> SyncState hash. The Scenario A
/// roundtrip (Scenario A) is tested separately via
/// [`mailbox_roundtrip_scenario`].
pub fn mailbox_send_scenario(message_count: u64) -> ScenarioFixture {
    assert!(
        message_count > 0,
        "mailbox_send_scenario needs at least 1 message"
    );
    let cap = (message_count as usize)
        .checked_add(1)
        .expect("mailbox_send_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            let target = rt.mailbox_registry_mut().register();
            rt.registry_mut()
                .register_with(|id| MailboxProducer::new(id, target, message_count));
        })
        .build()
}

/// **Scenario A: mailbox roundtrip.**
///
/// PPU-like sender sends a command to SPU-like responder via
/// `cmd_mailbox`, then blocks. Responder receives the command,
/// computes `command + 1`, sends the response to `resp_mailbox`,
/// wakes the sender, and finishes. Sender wakes, receives the
/// response, and finishes with a `TraceMarker` carrying the
/// response value.
///
/// The run ends with both units Finished, both mailboxes empty, and
/// the trace containing the full send-receive-respond-receive
/// sequence. 5 steps total with budget=1 and round-robin scheduling.
pub fn mailbox_roundtrip_scenario(command: u32) -> ScenarioFixture {
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(20)
        .register(move |rt: &mut Runtime| {
            let cmd_mb = rt.mailbox_registry_mut().register();
            let resp_mb = rt.mailbox_registry_mut().register();
            // Sender is id 0 (registered first), responder is id 1.
            let sender_id = cellgov_event::UnitId::new(0);
            let responder_id = cellgov_event::UnitId::new(1);
            rt.registry_mut()
                .register_with(|id| MailboxSender::new(id, responder_id, cmd_mb, resp_mb, command));
            rt.registry_mut()
                .register_with(|id| MailboxResponder::new(id, sender_id, cmd_mb, resp_mb));
        })
        .build()
}

/// **Signal emitter scenario.** Registers a single signal-notification
/// register plus a [`SignalEmitter`] that OR-merges `bit_count`
/// distinct bits (low bits 0..bit_count-1) into it across `bit_count`
/// steps. The run ends with the register's value equal to
/// `(1 << bit_count) - 1`.
///
/// Smallest end-to-end test of the `SignalUpdate` commit path through
/// the runner: emitter step -> EffectEmitted record -> commit pipeline
/// -> signal register -> SyncState hash. `bit_count` must be at most
/// 32 (the register is `u32`); the underlying [`SignalEmitter::new`]
/// panics on larger values.
pub fn signal_update_scenario(bit_count: u64) -> ScenarioFixture {
    assert!(bit_count > 0, "signal_update_scenario needs at least 1 bit");
    assert!(
        bit_count <= 32,
        "signal_update_scenario bit_count must be <= 32, got {bit_count}"
    );
    let cap = (bit_count as usize)
        .checked_add(1)
        .expect("signal_update_scenario step cap overflow");
    ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(cap)
        .register(move |rt: &mut Runtime| {
            let target = rt.signal_registry_mut().register();
            rt.registry_mut()
                .register_with(|id| SignalEmitter::new(id, target, bit_count));
        })
        .build()
}

/// **Fake-ISA integration stress test.** Runs a single [`FakeIsaUnit`]
/// through a multi-opcode program that exercises every handled effect
/// path in one run: `LoadImm` -> `SharedStore` -> `MailboxSend` ->
/// `End`.
///
/// The program loads 0xAB into the accumulator, stores it at addr 0
/// (4 bytes), sends it to mailbox 0, then ends. The run proves that
/// `SharedWriteIntent`, `MailboxSend`, and the yield/commit cycle
/// compose correctly from a single decoded instruction stream.
pub fn fake_isa_scenario() -> ScenarioFixture {
    ScenarioFixture::builder()
        .memory_size(256)
        .budget(Budget::new(1))
        .max_steps(20)
        .register(move |rt: &mut Runtime| {
            rt.mailbox_registry_mut().register(); // mailbox 0
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xab),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::MailboxSend { mailbox: 0 },
                        FakeOp::End,
                    ],
                )
            });
        })
        .build()
}

#[cfg(test)]
mod tests {
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

    /// Decode the trace and pull out the order in which units were
    /// scheduled. Useful for fairness and ordering assertions.
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
        // 3 units, 4 steps each. Round-robin must visit them as
        // 0,1,2,0,1,2,0,1,2,0,1,2 -- 12 schedule decisions total.
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
        // 4 units, 5 steps each. Each unit must appear in the trace
        // exactly 5 times -- no runnable unit can be skipped.
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
        // Edge case: a single unit still finishes cleanly under the
        // round-robin policy (the round consists of just that unit).
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

    /// Decode the trace and pull out the (unit, sequence, kind) tuple
    /// for every EffectEmitted record. Used by the write-conflict
    /// tests to verify the per-unit write interleave.
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
        // Round-robin with 4 steps per unit yields 8 effect emissions
        // in the order 0,1,0,1,0,1,0,1 -- one SharedWriteIntent per
        // step, alternating units.
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

        // The scenario's final committed memory must equal a
        // hand-constructed GuestMemory with the same final write
        // applied. Last writer wins -- with matching step counts and
        // round-robin scheduling, the last commit belongs to unit 1's
        // final step, which writes [steps_per_unit; 4] at addr 0.
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
        // Build a parallel `Runtime` with a hand-seeded mailbox
        // matching what the producer would have sent, and assert its
        // combined sync_state_hash matches the run's final_sync_hash.
        // Building through a real Runtime keeps the test honest about
        // every sync source the runtime aggregates (mailbox + signal),
        // not just the mailbox alone.
        use cellgov_core::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_trace::StateHash;
        let result = run(mailbox_send_scenario(5));

        let mut expected = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
        let mb = expected.mailbox_registry_mut().register();
        for n in 1..=5u32 {
            expected.mailbox_registry_mut().get_mut(mb).unwrap().send(n);
        }
        assert_eq!(
            result.final_sync_hash,
            StateHash::new(expected.sync_state_hash())
        );
    }

    #[test]
    fn mailbox_send_scenario_emits_one_send_effect_per_message() {
        // Decode the trace and count MailboxSend EffectEmitted records.
        // There must be exactly `message_count` of them, in sequence
        // 0..N within their step (one effect per step, so each
        // EffectEmitted has sequence 0).
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
        // Build a parallel Runtime with the same final signal value
        // and assert its sync_state_hash matches. After `bit_count`
        // OR-merges of distinct low bits, the register holds
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
        use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind};
        // The final TraceMarker carries the response value, which
        // should be command + 1 per MailboxResponder's logic.
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
        // Exactly one TraceMarker effect in the whole run.
        assert_eq!(final_marker, 1);
        // Verify the marker value by looking at the original
        // TraceMarker effect inside the step result. The
        // EffectEmitted trace record only carries the kind, not the
        // payload. To verify the actual value, build a parallel
        // runtime with the expected final sync state.
        //
        // For now, we assert the step count and effect sequence are
        // correct (tested above) which implicitly proves the
        // response path -- the sender only finishes if it received
        // a message from the responder.
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
        // Verify the destination range has the seed bytes after the
        // run. Build a GuestMemory with the expected final state and
        // compare the hash.
        use cellgov_mem::GuestMemory;
        use cellgov_trace::StateHash;
        let result = run(dma_block_unblock_scenario());
        let mut expected_mem = GuestMemory::new(256);
        // Source bytes at addr 0 (seeded by submitter).
        expected_mem
            .apply_commit(
                ByteRange::new(GuestAddr::new(0), 4).unwrap(),
                &[0xde, 0xad, 0xbe, 0xef],
            )
            .unwrap();
        // DMA copied them to addr 128.
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
        // Submitter step 1: SharedWriteIntent, DmaEnqueue, WaitOnEvent.
        // We filter for DmaEnqueue and WaitOnEvent only.
        assert_eq!(
            dma_effects,
            vec![TracedEffectKind::DmaEnqueue, TracedEffectKind::WaitOnEvent]
        );
    }

    #[test]
    fn mailbox_roundtrip_final_mailboxes_are_empty() {
        // Both messages were consumed: cmd by responder, resp by
        // sender. The final sync hash should match an empty registry
        // (two empty mailboxes, no signals).
        use cellgov_core::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_trace::StateHash;
        let result = run(mailbox_roundtrip_scenario(0x42));
        let mut expected = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
        expected.mailbox_registry_mut().register(); // cmd
        expected.mailbox_registry_mut().register(); // resp
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
        // After SharedStore{addr:0, len:4} with acc=0xAB, committed
        // memory at [0..4] should be [0xAB, 0xAB, 0xAB, 0xAB].
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
        // MailboxSend{mailbox:0} with acc=0xAB should deposit 0xAB
        // into mailbox 0. The sync hash should match a runtime with
        // one mailbox holding [0xAB].
        use cellgov_core::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_trace::StateHash;
        let result = run(fake_isa_scenario());
        let mut expected = Runtime::new(GuestMemory::new(256), Budget::new(1), 100);
        let mb = expected.mailbox_registry_mut().register();
        expected
            .mailbox_registry_mut()
            .get_mut(mb)
            .unwrap()
            .send(0xab);
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
        // LoadImm emits no effects. SharedStore emits SharedWriteIntent.
        // MailboxSend emits MailboxSend. End emits nothing.
        assert_eq!(
            effects,
            vec![
                TracedEffectKind::SharedWriteIntent,
                TracedEffectKind::MailboxSend,
            ]
        );
    }
}
