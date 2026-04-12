use super::*;
use cellgov_dma::{DmaDirection, DmaQueue, DmaRequest, FixedLatency};
use cellgov_effects::{FaultKind, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::LocalDiagnostics;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

// Inline Dummy/RunnableUnit/BlockedUnit structs inside individual tests
// are local test doubles. cellgov_testkit depends on cellgov_core, so a
// reverse dev-dependency would create a cycle.

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).unwrap()
}

fn write_intent(start: u64, bytes: Vec<u8>) -> Effect {
    Effect::SharedWriteIntent {
        range: range(start, bytes.len() as u64),
        bytes: WritePayload::new(bytes),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    }
}

fn marker() -> Effect {
    Effect::TraceMarker {
        marker: 1,
        source: UnitId::new(0),
    }
}

fn step_with(yield_reason: YieldReason, effects: Vec<Effect>) -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason,
        consumed_budget: Budget::new(1),
        emitted_effects: effects,
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
    }
}

#[test]
fn empty_step_is_noop() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let r = step_with(YieldReason::BudgetExhausted, vec![]);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 0);
    assert!(!outcome.fault_discarded);
}

#[test]
fn single_shared_write_becomes_visible() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0, vec![1, 2, 3, 4])],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
}

#[test]
fn multiple_shared_writes_apply_in_emission_order() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 1, 1, 1]),
            write_intent(2, vec![2, 2, 2, 2]),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 2);
    // Last writer wins on the overlap.
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
}

#[test]
fn non_write_effects_are_deferred() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![marker(), write_intent(0, vec![9, 9]), marker()],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.effects_deferred, 2);
    assert_eq!(mem.read(range(0, 2)).unwrap(), &[9, 9]);
}

#[test]
fn fault_step_discards_everything() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    // The "step" has writes but yields with Fault -- nothing should
    // become visible.
    let mut r = step_with(YieldReason::Fault, vec![write_intent(0, vec![7, 7, 7, 7])]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn payload_length_mismatch_aborts_batch_atomically() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    // First effect is valid, second has a mismatched payload.
    let bad = Effect::SharedWriteIntent {
        range: range(4, 4),
        bytes: WritePayload::new(vec![9, 9]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    };
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0, vec![1, 1, 1, 1]), bad],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(err, CommitError::PayloadLengthMismatch { effect_index: 1 });
    // Memory left untouched -- the valid first effect was staged
    // but the staging buffer is dropped without draining.
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn out_of_range_aborts_batch_atomically() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    // 6+4 = 10 > 8 -> out of range.
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 1, 1, 1]),
            write_intent(6, vec![2, 2, 2, 2]),
        ],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(err, CommitError::OutOfRange { effect_index: 1 });
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn fault_step_with_no_effects() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut r = step_with(YieldReason::Fault, vec![]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 0);
}

fn mailbox_send(mailbox: MailboxId, message: u32) -> Effect {
    use cellgov_effects::MailboxMessage;
    Effect::MailboxSend {
        mailbox,
        message: MailboxMessage::new(message),
        source: UnitId::new(0),
    }
}

#[test]
fn mailbox_send_pushes_message_into_registry() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    let r = step_with(YieldReason::BudgetExhausted, vec![mailbox_send(mb, 42)]);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(mailboxes.get(mb).unwrap().peek(), Some(42));
}

#[test]
fn multiple_mailbox_sends_apply_in_emission_order() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            mailbox_send(mb, 1),
            mailbox_send(mb, 2),
            mailbox_send(mb, 3),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.mailbox_sends_committed, 3);
    let m = mailboxes.get_mut(mb).unwrap();
    assert_eq!(m.try_receive(), Some(1));
    assert_eq!(m.try_receive(), Some(2));
    assert_eq!(m.try_receive(), Some(3));
}

#[test]
fn unknown_mailbox_aborts_batch_atomically() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let known = mailboxes.register();
    let unknown = MailboxId::new(99);
    // First send is valid, second references an unregistered mailbox.
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![mailbox_send(known, 7), mailbox_send(unknown, 8)],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownMailbox {
            effect_index: 1,
            mailbox: unknown
        }
    );
    // The valid first send must NOT have been applied -- the
    // batch is atomic, all-or-nothing.
    assert!(mailboxes.get(known).unwrap().is_empty());
}

#[test]
fn writes_and_mailbox_sends_compose_in_one_step() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![0xaa, 0xbb, 0xcc, 0xdd]),
            mailbox_send(mb, 0xcafe),
            marker(),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.effects_deferred, 1); // the marker
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[0xaa, 0xbb, 0xcc, 0xdd]);
    assert_eq!(mailboxes.get(mb).unwrap().peek(), Some(0xcafe));
}

fn dma_enqueue_effect(src: u64, dst: u64, len: u64) -> Effect {
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(src), len).unwrap(),
        ByteRange::new(GuestAddr::new(dst), len).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    Effect::DmaEnqueue { request: req }
}

#[test]
fn dma_enqueue_schedules_into_queue() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(256);
    let mut dma_queue = DmaQueue::new();
    let latency = FixedLatency::new(10);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![dma_enqueue_effect(0, 128, 16)],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut dma_queue,
            &latency,
            GuestTicks::new(100),
        )
        .unwrap();
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(dma_queue.len(), 1);
    let c = dma_queue.peek().unwrap();
    // FixedLatency(10) at now=100 => completion at 110.
    assert_eq!(c.completion_time(), GuestTicks::new(110));
    assert_eq!(c.length(), 16);
}

#[test]
fn multiple_dma_enqueues_schedule_in_emission_order() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(256);
    let mut dma_queue = DmaQueue::new();
    let latency = FixedLatency::new(5);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![dma_enqueue_effect(0, 128, 8), dma_enqueue_effect(8, 136, 8)],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut dma_queue,
            &latency,
            GuestTicks::new(50),
        )
        .unwrap();
    assert_eq!(outcome.dma_enqueued, 2);
    assert_eq!(dma_queue.len(), 2);
    // Both at same completion time; pop order is enqueue order.
    let c1 = dma_queue.pop_next().unwrap();
    let c2 = dma_queue.pop_next().unwrap();
    assert_eq!(c1.completion_time(), GuestTicks::new(55));
    assert_eq!(c2.completion_time(), GuestTicks::new(55));
    assert_eq!(c1.source().start().raw(), 0);
    assert_eq!(c2.source().start().raw(), 8);
}

#[test]
fn fault_step_discards_dma_enqueues() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(256);
    let mut dma_queue = DmaQueue::new();
    let latency = FixedLatency::new(10);
    let mut r = step_with(YieldReason::Fault, vec![dma_enqueue_effect(0, 128, 16)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut dma_queue,
            &latency,
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.dma_enqueued, 0);
    assert!(dma_queue.is_empty());
}

#[test]
fn all_four_handled_effects_compose_in_one_step() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(256);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let mut dma_queue = DmaQueue::new();
    let latency = FixedLatency::new(10);
    let mb = mailboxes.register();
    let sig = signals.register();
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 2, 3, 4]),
            mailbox_send(mb, 0xfeed),
            signal_update(sig, 0xa5),
            dma_enqueue_effect(0, 128, 8),
            marker(),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut dma_queue,
            &latency,
            GuestTicks::new(200),
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(outcome.effects_deferred, 1); // the marker
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(mailboxes.get(mb).unwrap().peek(), Some(0xfeed));
    assert_eq!(signals.get(sig).unwrap().value(), 0xa5);
    assert_eq!(
        dma_queue.peek().unwrap().completion_time(),
        GuestTicks::new(210)
    );
}

fn mailbox_receive(mailbox: MailboxId, source: UnitId) -> Effect {
    Effect::MailboxReceiveAttempt { mailbox, source }
}

#[test]
fn receive_from_non_empty_mailbox_pops_and_delivers() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    let receiver_id = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct Dummy(UnitId);
        impl ExecutionUnit for Dummy {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Runnable
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        Dummy(id)
    });
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    mailboxes.get_mut(mb).unwrap().send(0xdead);
    mailboxes.get_mut(mb).unwrap().send(0xbeef);
    let r = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(mb, receiver_id)],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.mailbox_receives_committed, 1);
    assert_eq!(outcome.mailbox_receives_blocked, 0);
    // One message popped (0xdead), one remains (0xbeef).
    assert_eq!(mailboxes.get(mb).unwrap().len(), 1);
    // Delivered to unit's pending receives.
    let delivered = units.drain_receives(receiver_id);
    assert_eq!(delivered, vec![0xdead]);
    // Unit still runnable (not blocked).
    assert_eq!(
        units.effective_status(receiver_id),
        Some(UnitStatus::Runnable)
    );
}

#[test]
fn receive_from_empty_mailbox_blocks_unit() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    let receiver_id = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct Dummy(UnitId);
        impl ExecutionUnit for Dummy {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Runnable
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::MailboxAccess,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        Dummy(id)
    });
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    // Mailbox is empty.
    let r = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(mb, receiver_id)],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.mailbox_receives_committed, 0);
    assert_eq!(outcome.mailbox_receives_blocked, 1);
    assert_eq!(
        units.effective_status(receiver_id),
        Some(UnitStatus::Blocked)
    );
}

#[test]
fn receive_from_unknown_mailbox_aborts_batch() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let unknown = MailboxId::new(99);
    let r = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(unknown, UnitId::new(0))],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownMailbox {
            effect_index: 0,
            mailbox: unknown
        }
    );
}

fn wait_effect(source: UnitId) -> Effect {
    use cellgov_effects::WaitTarget;
    use cellgov_sync::MailboxId;
    Effect::WaitOnEvent {
        target: WaitTarget::Mailbox(MailboxId::new(0)),
        source,
    }
}

#[test]
fn wait_on_event_blocks_the_source_unit() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    // Register a unit that self-reports Runnable.
    let id = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct RunnableUnit(UnitId);
        impl ExecutionUnit for RunnableUnit {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Runnable
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        RunnableUnit(id)
    });
    assert_eq!(units.effective_status(id), Some(UnitStatus::Runnable));
    let r = step_with(YieldReason::WaitingSync, vec![wait_effect(id)]);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.waits_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(units.effective_status(id), Some(UnitStatus::Blocked));
}

#[test]
fn fault_step_discards_wait_effects() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    let id = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct Dummy(UnitId);
        impl ExecutionUnit for Dummy {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Runnable
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        Dummy(id)
    });
    let mut r = step_with(YieldReason::Fault, vec![wait_effect(id)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.waits_committed, 0);
    // Still self-reports Runnable -- fault discarded the wait.
    assert_eq!(units.effective_status(id), Some(UnitStatus::Runnable));
}

#[test]
fn wait_then_wake_restores_runnable() {
    // A single commit with both WaitOnEvent and WakeUnit on the
    // same target. Effects apply in emission order: wait blocks,
    // then wake unblocks. Net result is Runnable.
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    let id = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct Dummy(UnitId);
        impl ExecutionUnit for Dummy {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Runnable
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        Dummy(id)
    });
    let r = step_with(
        YieldReason::WaitingSync,
        vec![wait_effect(id), wake_effect(id)],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.waits_committed, 1);
    assert_eq!(outcome.wakes_committed, 1);
    // Wait set Blocked, then Wake set Runnable. Net: Runnable.
    assert_eq!(units.effective_status(id), Some(UnitStatus::Runnable));
}

fn wake_effect(target: UnitId) -> Effect {
    Effect::WakeUnit {
        target,
        source: UnitId::new(99),
    }
}

#[test]
fn wake_unit_sets_status_override_to_runnable() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    let target = units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct BlockedUnit(UnitId);
        impl ExecutionUnit for BlockedUnit {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Blocked
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        BlockedUnit(id)
    });
    // Before commit: effective status is Blocked (self-reported).
    assert_eq!(units.effective_status(target), Some(UnitStatus::Blocked));
    let r = step_with(YieldReason::BudgetExhausted, vec![wake_effect(target)]);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.wakes_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    // After commit: override set to Runnable.
    assert_eq!(units.effective_status(target), Some(UnitStatus::Runnable));
}

#[test]
fn unknown_wake_target_aborts_batch() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![wake_effect(UnitId::new(99))],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownWakeTarget {
            effect_index: 0,
            target: UnitId::new(99)
        }
    );
}

#[test]
fn fault_step_discards_wake_effects() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut units = UnitRegistry::new();
    units.register_with(|id| {
        use cellgov_exec::{ExecutionContext, ExecutionUnit, LocalDiagnostics};
        struct Dummy(UnitId);
        impl ExecutionUnit for Dummy {
            type Snapshot = ();
            fn unit_id(&self) -> UnitId {
                self.0
            }
            fn status(&self) -> UnitStatus {
                UnitStatus::Blocked
            }
            fn run_until_yield(
                &mut self,
                b: Budget,
                _: &ExecutionContext<'_>,
            ) -> cellgov_exec::ExecutionStepResult {
                cellgov_exec::ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_budget: b,
                    emitted_effects: vec![],
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                }
            }
            fn snapshot(&self) {}
        }
        Dummy(id)
    });
    let mut r = step_with(YieldReason::Fault, vec![wake_effect(UnitId::new(0))]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut units,
            &mut MailboxRegistry::new(),
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.wakes_committed, 0);
    // No override set -- still self-reports Blocked.
    assert_eq!(
        units.effective_status(UnitId::new(0)),
        Some(UnitStatus::Blocked)
    );
}

fn signal_update(signal: SignalId, value: u32) -> Effect {
    Effect::SignalUpdate {
        signal,
        value,
        source: UnitId::new(0),
    }
}

#[test]
fn signal_update_or_merges_into_register() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let sig = signals.register();
    let r = step_with(YieldReason::BudgetExhausted, vec![signal_update(sig, 0x0f)]);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(signals.get(sig).unwrap().value(), 0x0f);
}

#[test]
fn multiple_signal_updates_or_merge_in_emission_order() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let sig = signals.register();
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            signal_update(sig, 0x01),
            signal_update(sig, 0x10),
            signal_update(sig, 0x100),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.signal_updates_committed, 3);
    assert_eq!(signals.get(sig).unwrap().value(), 0x111);
}

#[test]
fn unknown_signal_aborts_batch_atomically() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let known = signals.register();
    let unknown = SignalId::new(99);
    // First update is valid, second references an unknown signal.
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![signal_update(known, 0x1), signal_update(unknown, 0x2)],
    );
    let err = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownSignal {
            effect_index: 1,
            signal: unknown
        }
    );
    // The valid first update must NOT have been applied -- the
    // batch is atomic, all-or-nothing.
    assert_eq!(signals.get(known).unwrap().value(), 0);
}

#[test]
fn writes_mailbox_sends_and_signal_updates_compose_in_one_step() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let mb = mailboxes.register();
    let sig = signals.register();
    let r = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 2, 3, 4]),
            mailbox_send(mb, 0xfeed),
            signal_update(sig, 0xa5),
            marker(),
        ],
    );
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.effects_deferred, 1); // the marker
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(mailboxes.get(mb).unwrap().peek(), Some(0xfeed));
    assert_eq!(signals.get(sig).unwrap().value(), 0xa5);
}

#[test]
fn fault_step_discards_signal_updates() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mut signals = SignalRegistry::new();
    let sig = signals.register();
    let mut r = step_with(YieldReason::Fault, vec![signal_update(sig, 0xff)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut signals,
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.signal_updates_committed, 0);
    assert_eq!(signals.get(sig).unwrap().value(), 0);
}

#[test]
fn fault_step_discards_mailbox_sends() {
    let mut p = CommitPipeline::new();
    let mut mem = GuestMemory::new(8);
    let mut mailboxes = MailboxRegistry::new();
    let mb = mailboxes.register();
    let mut r = step_with(YieldReason::Fault, vec![mailbox_send(mb, 1)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = p
        .process(
            &r,
            &mut mem,
            &mut UnitRegistry::new(),
            &mut mailboxes,
            &mut SignalRegistry::new(),
            &mut DmaQueue::new(),
            &FixedLatency::new(10),
            GuestTicks::ZERO,
        )
        .unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.mailbox_sends_committed, 0);
    assert!(mailboxes.get(mb).unwrap().is_empty());
}
