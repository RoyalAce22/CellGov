//! Commit-pipeline effect application and fault-discard atomicity across every effect kind.

use super::*;
use cellgov_dma::{DmaDirection, DmaQueue, DmaRequest, FixedLatency};
use cellgov_effects::{FaultKind, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::LocalDiagnostics;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::ReservationTable;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

// cellgov_testkit depends on cellgov_core; local test doubles avoid the cycle.

pub(super) struct CommitTestBed {
    pipeline: CommitPipeline,
    mem: GuestMemory,
    pub(super) units: UnitRegistry,
    mailboxes: MailboxRegistry,
    signals: SignalRegistry,
    dma_queue: DmaQueue,
    latency: FixedLatency,
    now: GuestTicks,
    reservations: ReservationTable,
    /// `CommitContext::rsx_label_base` for the next `process` call.
    label_base: u32,
}

impl CommitTestBed {
    pub(super) fn new(mem_size: usize) -> Self {
        Self {
            pipeline: CommitPipeline::new(),
            mem: GuestMemory::new(mem_size),
            units: UnitRegistry::new(),
            mailboxes: MailboxRegistry::new(),
            signals: SignalRegistry::new(),
            dma_queue: DmaQueue::new(),
            latency: FixedLatency::new(10),
            now: GuestTicks::ZERO,
            reservations: ReservationTable::new(),
            label_base: 0,
        }
    }

    /// Bed over a caller-built region map, for a case that needs an
    /// access mode `GuestMemory::new` does not give.
    pub(super) fn with_memory(mem: GuestMemory) -> Self {
        Self {
            mem,
            ..Self::new(0)
        }
    }

    pub(super) fn memory(&self) -> &GuestMemory {
        &self.mem
    }

    pub(super) fn process(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        let mut flip = crate::rsx::flip::RsxFlipState::new();
        let mut label_writes = 0u64;
        let mut ctx = CommitContext {
            memory: &mut self.mem,
            dma_memory: None,
            units: &mut self.units,
            mailboxes: &mut self.mailboxes,
            signals: &mut self.signals,
            dma_queue: &mut self.dma_queue,
            dma_latency: &self.latency,
            now: self.now,
            reservations: &mut self.reservations,
            rsx_label_base: self.label_base,
            rsx_flip: &mut flip,
            rsx_label_writes_committed: &mut label_writes,
            tap: None,
        };
        self.pipeline.process(result, effects, &mut ctx)
    }
}

#[derive(Clone)]

pub(super) struct DummyUnit {
    id: UnitId,
    status: UnitStatus,
}

impl DummyUnit {
    pub(super) fn runnable(id: UnitId) -> Self {
        Self {
            id,
            status: UnitStatus::Runnable,
        }
    }

    fn blocked(id: UnitId) -> Self {
        Self {
            id,
            status: UnitStatus::Blocked,
        }
    }
}

impl cellgov_exec::ExecutionUnit for DummyUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        self.status
    }

    fn run_until_yield(
        &mut self,
        b: Budget,
        _: &cellgov_exec::ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> cellgov_exec::ExecutionStepResult {
        cellgov_exec::ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(b.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

pub(super) fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).unwrap()
}

/// Emitter for write intents whose source the test does not care
/// about.
///
/// Sits outside the range [`UnitRegistry`] allocates from, so it never
/// matches a registered unit and never takes the same-processor branch
/// of the reservation sweep. Tests that care about the source pass
/// their own id to [`write_intent_from`].
const UNSPECIFIED_SOURCE: UnitId = UnitId::new(0xD0);

fn write_intent(start: u64, bytes: Vec<u8>) -> Effect {
    write_intent_from(start, bytes, UNSPECIFIED_SOURCE)
}

fn write_intent_from(start: u64, bytes: Vec<u8>, source: UnitId) -> Effect {
    Effect::shared_write(
        range(start, bytes.len() as u64),
        WritePayload::new(bytes),
        source,
        GuestTicks::new(0),
    )
}

fn marker() -> Effect {
    Effect::TraceMarker {
        marker: 1,
        source: UnitId::new(0),
    }
}

pub(super) fn step_with(
    yield_reason: YieldReason,
    effects: Vec<Effect>,
) -> (ExecutionStepResult, Vec<Effect>) {
    (
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::ONE,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        },
        effects,
    )
}

#[test]
fn empty_step_is_noop() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![]);
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 0);
    assert!(!outcome.fault_discarded);
}

#[test]
fn single_shared_write_becomes_visible() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0, vec![1, 2, 3, 4])],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(bed.mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
}

#[test]
fn multiple_shared_writes_apply_in_emission_order() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 1, 1, 1]),
            write_intent(2, vec![2, 2, 2, 2]),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 2);
    assert_eq!(
        bed.mem.read(range(0, 8)).unwrap(),
        &[1, 1, 2, 2, 2, 2, 0, 0]
    );
}

#[test]
fn non_write_effects_are_deferred() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![marker(), write_intent(0, vec![9, 9]), marker()],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.effects_deferred, 2);
    assert_eq!(bed.mem.read(range(0, 2)).unwrap(), &[9, 9]);
}

/// `ClockRead` and `SharedReadIntent` exist for the dependency
/// relation, so a deferred count is the whole of what the pipeline may
/// do with them.
#[test]
fn a_batch_of_declarations_alone_commits_nothing() {
    let mut bed = CommitTestBed::new(8);
    let before = bed.mem.read(range(0, 8)).unwrap().to_vec();
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            Effect::ClockRead {
                source: UnitId::new(0),
            },
            Effect::SharedReadIntent {
                range: range(0, 4),
                source: UnitId::new(0),
            },
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 2);
    assert!(!outcome.fault_discarded);
    assert_eq!(bed.mem.read(range(0, 8)).unwrap(), &before[..]);
}

#[test]
fn fault_step_discards_everything() {
    let mut bed = CommitTestBed::new(8);
    let (mut r, e) = step_with(YieldReason::Fault, vec![write_intent(0, vec![7, 7, 7, 7])]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(bed.mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn payload_length_mismatch_aborts_batch_atomically() {
    let mut bed = CommitTestBed::new(8);
    let bad = Effect::shared_write(
        range(4, 4),
        WritePayload::new(vec![9, 9]),
        UnitId::new(0),
        GuestTicks::new(0),
    );
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0, vec![1, 1, 1, 1]), bad],
    );
    let err = bed.process(&r, &e).unwrap_err();
    assert_eq!(err, CommitError::PayloadLengthMismatch { effect_index: 1 });
    assert_eq!(bed.mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn out_of_range_aborts_batch_atomically() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 1, 1, 1]),
            write_intent(6, vec![2, 2, 2, 2]),
        ],
    );
    let err = bed.process(&r, &e).unwrap_err();
    assert_eq!(err, CommitError::OutOfRange { effect_index: 1 });
    assert_eq!(bed.mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn fault_step_with_no_effects() {
    let mut bed = CommitTestBed::new(8);
    let (mut r, e) = step_with(YieldReason::Fault, vec![]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
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
    let mut bed = CommitTestBed::new(8);
    let mb = bed.mailboxes.register(4);
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![mailbox_send(mb, 42)]);
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.writes_committed, 0);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(bed.mailboxes.get(mb).unwrap().peek(), Some(42));
}

#[test]
fn multiple_mailbox_sends_apply_in_emission_order() {
    let mut bed = CommitTestBed::new(8);
    let mb = bed.mailboxes.register(4);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            mailbox_send(mb, 1),
            mailbox_send(mb, 2),
            mailbox_send(mb, 3),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.mailbox_sends_committed, 3);
    let m = bed.mailboxes.get_mut(mb).unwrap();
    assert_eq!(m.try_receive(), Some(1));
    assert_eq!(m.try_receive(), Some(2));
    assert_eq!(m.try_receive(), Some(3));
}

#[test]
fn unknown_mailbox_aborts_batch_atomically() {
    let mut bed = CommitTestBed::new(8);
    let known = bed.mailboxes.register(4);
    let unknown = MailboxId::new(99);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![mailbox_send(known, 7), mailbox_send(unknown, 8)],
    );
    let err = bed.process(&r, &e).unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownMailbox {
            effect_index: 1,
            mailbox: unknown
        }
    );
    assert!(bed.mailboxes.get(known).unwrap().is_empty());
}

#[test]
fn writes_and_mailbox_sends_compose_in_one_step() {
    let mut bed = CommitTestBed::new(8);
    let mb = bed.mailboxes.register(4);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![0xaa, 0xbb, 0xcc, 0xdd]),
            mailbox_send(mb, 0xcafe),
            marker(),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.effects_deferred, 1);
    assert_eq!(
        bed.mem.read(range(0, 4)).unwrap(),
        &[0xaa, 0xbb, 0xcc, 0xdd]
    );
    assert_eq!(bed.mailboxes.get(mb).unwrap().peek(), Some(0xcafe));
}

fn dma_enqueue_effect(src: u64, dst: u64, len: u64) -> Effect {
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(src), len).unwrap(),
        ByteRange::new(GuestAddr::new(dst), len).unwrap(),
        UnitId::new(0),
    )
    .unwrap();
    Effect::DmaEnqueue {
        request: req,
        payload: None,
    }
}

#[test]
fn dma_enqueue_schedules_into_queue() {
    let mut bed = CommitTestBed::new(256);
    bed.now = GuestTicks::new(100);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![dma_enqueue_effect(0, 128, 16)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(bed.dma_queue.len(), 1);
    let c = bed.dma_queue.peek().unwrap();
    // FixedLatency(10) at now=100 => completion at 110.
    assert_eq!(c.completion_time(), GuestTicks::new(110));
    assert_eq!(c.length(), 16);
}

#[test]
fn multiple_dma_enqueues_schedule_in_emission_order() {
    let mut bed = CommitTestBed::new(256);
    bed.latency = FixedLatency::new(5);
    bed.now = GuestTicks::new(50);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![dma_enqueue_effect(0, 128, 8), dma_enqueue_effect(8, 136, 8)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.dma_enqueued, 2);
    assert_eq!(bed.dma_queue.len(), 2);
    // Both at same completion time; pop order is enqueue order.
    let (c1, _) = bed.dma_queue.pop_next().unwrap();
    let (c2, _) = bed.dma_queue.pop_next().unwrap();
    assert_eq!(c1.completion_time(), GuestTicks::new(55));
    assert_eq!(c2.completion_time(), GuestTicks::new(55));
    assert_eq!(c1.source().start().raw(), 0);
    assert_eq!(c2.source().start().raw(), 8);
}

#[test]
fn fault_step_discards_dma_enqueues() {
    let mut bed = CommitTestBed::new(256);
    let (mut r, e) = step_with(YieldReason::Fault, vec![dma_enqueue_effect(0, 128, 16)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.dma_enqueued, 0);
    assert!(bed.dma_queue.is_empty());
}

#[test]
fn all_four_handled_effects_compose_in_one_step() {
    let mut bed = CommitTestBed::new(256);
    bed.now = GuestTicks::new(200);
    let mb = bed.mailboxes.register(4);
    let sig = bed.signals.register();
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 2, 3, 4]),
            mailbox_send(mb, 0xfeed),
            signal_update(sig, 0xa5),
            dma_enqueue_effect(0, 128, 8),
            marker(),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(outcome.effects_deferred, 1); // the marker
    assert_eq!(bed.mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(bed.mailboxes.get(mb).unwrap().peek(), Some(0xfeed));
    assert_eq!(bed.signals.get(sig).unwrap().value(), 0xa5);
    assert_eq!(
        bed.dma_queue.peek().unwrap().completion_time(),
        GuestTicks::new(210)
    );
}

fn mailbox_receive(mailbox: MailboxId, source: UnitId) -> Effect {
    Effect::MailboxReceiveAttempt { mailbox, source }
}

#[test]
fn receive_from_non_empty_mailbox_pops_and_delivers() {
    let mut bed = CommitTestBed::new(8);
    let receiver_id = bed.units.register_with(DummyUnit::runnable);
    let mb = bed.mailboxes.register(4);
    bed.mailboxes.get_mut(mb).unwrap().force_send(0xdead);
    bed.mailboxes.get_mut(mb).unwrap().force_send(0xbeef);
    let (r, e) = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(mb, receiver_id)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.mailbox_receives_committed, 1);
    assert_eq!(outcome.mailbox_receives_blocked, 0);
    // One message popped (0xdead), one remains (0xbeef).
    assert_eq!(bed.mailboxes.get(mb).unwrap().len(), 1);
    // Delivered to unit's pending receives.
    let delivered = bed.units.drain_receives(receiver_id);
    assert_eq!(delivered, vec![0xdead]);
    // Unit still runnable (not blocked).
    assert_eq!(
        bed.units.effective_status(receiver_id),
        Some(UnitStatus::Runnable)
    );
}

#[test]
fn receive_from_empty_mailbox_blocks_unit() {
    let mut bed = CommitTestBed::new(8);
    let receiver_id = bed.units.register_with(DummyUnit::runnable);
    let mb = bed.mailboxes.register(4);
    // Mailbox is empty.
    let (r, e) = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(mb, receiver_id)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.mailbox_receives_committed, 0);
    assert_eq!(outcome.mailbox_receives_blocked, 1);
    assert_eq!(
        bed.units.effective_status(receiver_id),
        Some(UnitStatus::Blocked)
    );
}

#[test]
fn receive_from_unknown_mailbox_aborts_batch() {
    let mut bed = CommitTestBed::new(8);
    let unknown = MailboxId::new(99);
    let (r, e) = step_with(
        YieldReason::MailboxAccess,
        vec![mailbox_receive(unknown, UnitId::new(0))],
    );
    let err = bed.process(&r, &e).unwrap_err();
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

fn barrier_wait(source: UnitId, barrier: u64) -> Effect {
    use cellgov_effects::WaitTarget;
    use cellgov_sync::BarrierId;
    Effect::WaitOnEvent {
        target: WaitTarget::Barrier(BarrierId::new(barrier)),
        source,
    }
}

/// A wait blocks its own source and frees nobody, whatever it names.
///
/// Schedule exploration depends on this: it prunes two units that wait
/// on different barriers as independent, which holds only while no
/// barrier releases a waiter.
#[test]
fn a_wait_names_a_target_the_pipeline_never_reads() {
    // Each commit carries one wait, so the two steps make the shape
    // the prune reasons about. A barrier that released its waiters
    // would appear as the second commit that frees the first unit.
    let statuses = |barriers: [u64; 2]| {
        let mut bed = CommitTestBed::new(8);
        let first = bed.units.register_with(DummyUnit::runnable);
        let second = bed.units.register_with(DummyUnit::runnable);
        for (unit, barrier) in [(first, barriers[0]), (second, barriers[1])] {
            let (r, e) = step_with(YieldReason::WaitingSync, vec![barrier_wait(unit, barrier)]);
            let outcome = bed.process(&r, &e).unwrap();
            assert_eq!(outcome.waits_committed, 1);
        }
        (
            bed.units.effective_status(first),
            bed.units.effective_status(second),
        )
    };
    let blocked = (Some(UnitStatus::Blocked), Some(UnitStatus::Blocked));
    assert_eq!(statuses([1, 2]), blocked);
    assert_eq!(statuses([1, 1]), blocked);
}

#[test]
fn wait_on_event_blocks_the_source_unit() {
    let mut bed = CommitTestBed::new(8);
    // Register a unit that self-reports Runnable.
    let id = bed.units.register_with(DummyUnit::runnable);
    assert_eq!(bed.units.effective_status(id), Some(UnitStatus::Runnable));
    let (r, e) = step_with(YieldReason::WaitingSync, vec![wait_effect(id)]);
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.waits_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(bed.units.effective_status(id), Some(UnitStatus::Blocked));
}

#[test]
fn fault_step_discards_wait_effects() {
    let mut bed = CommitTestBed::new(8);
    let id = bed.units.register_with(DummyUnit::runnable);
    let (mut r, e) = step_with(YieldReason::Fault, vec![wait_effect(id)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.waits_committed, 0);
    // Still self-reports Runnable -- fault discarded the wait.
    assert_eq!(bed.units.effective_status(id), Some(UnitStatus::Runnable));
}

#[test]
fn wait_then_wake_restores_runnable() {
    // A single commit with both WaitOnEvent and WakeUnit on the
    // same target. Effects apply in emission order: wait blocks,
    // then wake unblocks. Net result is Runnable.
    let mut bed = CommitTestBed::new(8);
    let id = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::WaitingSync,
        vec![wait_effect(id), wake_effect(id)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.waits_committed, 1);
    assert_eq!(outcome.wakes_committed, 1);
    // Wait set Blocked, then Wake set Runnable. Net: Runnable.
    assert_eq!(bed.units.effective_status(id), Some(UnitStatus::Runnable));
}

fn wake_effect(target: UnitId) -> Effect {
    Effect::WakeUnit {
        target,
        source: UnitId::new(99),
    }
}

#[test]
fn wake_unit_sets_status_override_to_runnable() {
    let mut bed = CommitTestBed::new(8);
    let target = bed.units.register_with(DummyUnit::blocked);
    // Before commit: effective status is Blocked (self-reported).
    assert_eq!(
        bed.units.effective_status(target),
        Some(UnitStatus::Blocked)
    );
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![wake_effect(target)]);
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.wakes_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    // After commit: override set to Runnable.
    assert_eq!(
        bed.units.effective_status(target),
        Some(UnitStatus::Runnable)
    );
}

#[test]
fn unknown_wake_target_aborts_batch() {
    let mut bed = CommitTestBed::new(8);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![wake_effect(UnitId::new(99))],
    );
    let err = bed.process(&r, &e).unwrap_err();
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
    let mut bed = CommitTestBed::new(8);
    bed.units.register_with(DummyUnit::blocked);
    let (mut r, e) = step_with(YieldReason::Fault, vec![wake_effect(UnitId::new(0))]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.wakes_committed, 0);
    // No override set -- still self-reports Blocked.
    assert_eq!(
        bed.units.effective_status(UnitId::new(0)),
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
    let mut bed = CommitTestBed::new(8);
    let sig = bed.signals.register();
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![signal_update(sig, 0x0f)]);
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.effects_deferred, 0);
    assert_eq!(bed.signals.get(sig).unwrap().value(), 0x0f);
}

#[test]
fn multiple_signal_updates_or_merge_in_emission_order() {
    let mut bed = CommitTestBed::new(8);
    let sig = bed.signals.register();
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            signal_update(sig, 0x01),
            signal_update(sig, 0x10),
            signal_update(sig, 0x100),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.signal_updates_committed, 3);
    assert_eq!(bed.signals.get(sig).unwrap().value(), 0x111);
}

#[test]
fn unknown_signal_aborts_batch_atomically() {
    let mut bed = CommitTestBed::new(8);
    let known = bed.signals.register();
    let unknown = SignalId::new(99);
    // First update is valid, second references an unknown signal.
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![signal_update(known, 0x1), signal_update(unknown, 0x2)],
    );
    let err = bed.process(&r, &e).unwrap_err();
    assert_eq!(
        err,
        CommitError::UnknownSignal {
            effect_index: 1,
            signal: unknown
        }
    );
    // The valid first update must not have been applied -- the
    // batch is atomic, all-or-nothing.
    assert_eq!(bed.signals.get(known).unwrap().value(), 0);
}

#[test]
fn writes_mailbox_sends_and_signal_updates_compose_in_one_step() {
    let mut bed = CommitTestBed::new(8);
    let mb = bed.mailboxes.register(4);
    let sig = bed.signals.register();
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            write_intent(0, vec![1, 2, 3, 4]),
            mailbox_send(mb, 0xfeed),
            signal_update(sig, 0xa5),
            marker(),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.mailbox_sends_committed, 1);
    assert_eq!(outcome.signal_updates_committed, 1);
    assert_eq!(outcome.effects_deferred, 1); // the marker
    assert_eq!(bed.mem.read(range(0, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(bed.mailboxes.get(mb).unwrap().peek(), Some(0xfeed));
    assert_eq!(bed.signals.get(sig).unwrap().value(), 0xa5);
}

#[test]
fn fault_step_discards_signal_updates() {
    let mut bed = CommitTestBed::new(8);
    let sig = bed.signals.register();
    let (mut r, e) = step_with(YieldReason::Fault, vec![signal_update(sig, 0xff)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.signal_updates_committed, 0);
    assert_eq!(bed.signals.get(sig).unwrap().value(), 0);
}

#[test]
fn fault_step_discards_mailbox_sends() {
    let mut bed = CommitTestBed::new(8);
    let mb = bed.mailboxes.register(4);
    let (mut r, e) = step_with(YieldReason::Fault, vec![mailbox_send(mb, 1)]);
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(outcome.mailbox_sends_committed, 0);
    assert!(bed.mailboxes.get(mb).unwrap().is_empty());
}

#[test]
fn two_receives_same_mailbox_first_pops_second_blocks() {
    let mut bed = CommitTestBed::new(8);
    let receiver_id = bed.units.register_with(DummyUnit::runnable);
    let mb = bed.mailboxes.register(4);
    // Put exactly one message in the mailbox.
    bed.mailboxes.get_mut(mb).unwrap().force_send(42);
    // Two receives from the same mailbox in one step.
    let (r, e) = step_with(
        YieldReason::MailboxAccess,
        vec![
            mailbox_receive(mb, receiver_id),
            mailbox_receive(mb, receiver_id),
        ],
    );
    let outcome = bed.process(&r, &e).unwrap();
    // First receive pops the message, second blocks.
    assert_eq!(outcome.mailbox_receives_committed, 1);
    assert_eq!(outcome.mailbox_receives_blocked, 1);
}

// Reservation-model helpers + tests.

fn reservation_acquire(line_addr: u64, source: UnitId) -> Effect {
    Effect::ReservationAcquire { line_addr, source }
}

fn conditional_store(addr: u64, bytes: Vec<u8>, source: UnitId) -> Effect {
    Effect::ConditionalStore {
        range: range(addr, bytes.len() as u64),
        bytes: WritePayload::new(bytes),
        ordering: PriorityClass::Normal,
        source,
        source_time: GuestTicks::new(0),
    }
}

#[test]
fn reservation_acquire_installs_entry() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![reservation_acquire(0x100, u)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.reservation_acquires_committed, 1);
    assert!(bed.reservations.is_held_by(u));
}

#[test]
fn reservation_acquire_canonicalizes_to_line() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        // Raw EA inside a line; table must store the aligned line.
        vec![reservation_acquire(0x140, u)],
    );
    bed.process(&r, &e).unwrap();
    assert_eq!(bed.reservations.get(u).unwrap().addr(), 0x100);
}

#[test]
fn reservation_acquire_replaces_prior_entry() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![reservation_acquire(0x100, u), reservation_acquire(0x200, u)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.reservation_acquires_committed, 2);
    assert_eq!(bed.reservations.get(u).unwrap().addr(), 0x200);
    // Replacing an existing entry counts as a clear under the new
    // outcome invariant (replay tooling sees the entry dropped).
    assert_eq!(outcome.reservations_cleared, 1);
    assert_eq!(bed.reservations.len(), 1);
}

#[test]
fn shared_write_clears_reservation_covering_line() {
    let mut bed = CommitTestBed::new(4096);
    // Pre-populate the table: unit 1 has a reservation on 0x100.
    bed.reservations.insert_or_replace(
        UnitId::new(1),
        cellgov_sync::ReservedLine::containing(0x100),
    );
    // Unit 0 now commits a plain store at 0x140 (same line).
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0x140, vec![0xAA, 0xBB, 0xCC, 0xDD])],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(outcome.reservations_cleared, 1);
    assert!(!bed.reservations.is_held_by(UnitId::new(1)));
}

#[test]
fn shared_write_leaves_non_overlapping_reservation_alone() {
    let mut bed = CommitTestBed::new(4096);
    bed.reservations.insert_or_replace(
        UnitId::new(1),
        cellgov_sync::ReservedLine::containing(0x200),
    );
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent(0x100, vec![0xAA, 0xBB, 0xCC, 0xDD])],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.reservations_cleared, 0);
    assert!(bed.reservations.is_held_by(UnitId::new(1)));
}

#[test]
fn conditional_store_applies_bytes_and_retires_own_reservation() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    bed.reservations
        .insert_or_replace(u, cellgov_sync::ReservedLine::containing(0x100));
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![conditional_store(0x100, vec![1, 2, 3, 4], u)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.conditional_stores_committed, 1);
    // Emitter's own entry counts as one cleared reservation.
    assert_eq!(outcome.reservations_cleared, 1);
    assert!(!bed.reservations.is_held_by(u));
    // Bytes landed.
    let read = bed.mem.read(range(0x100, 4)).unwrap();
    assert_eq!(read, &[1, 2, 3, 4]);
}

#[test]
fn conditional_store_also_clears_other_units_reservations_on_same_line() {
    let mut bed = CommitTestBed::new(4096);
    let winner = bed.units.register_with(DummyUnit::runnable);
    let loser = bed.units.register_with(DummyUnit::runnable);
    bed.reservations
        .insert_or_replace(winner, cellgov_sync::ReservedLine::containing(0x100));
    bed.reservations
        .insert_or_replace(loser, cellgov_sync::ReservedLine::containing(0x100));
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![conditional_store(0x140, vec![9; 4], winner)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    // Winner's own entry + loser's overlapping entry = 2 cleared.
    assert_eq!(outcome.reservations_cleared, 2);
    assert!(bed.reservations.is_empty());
}

/// [`UNSPECIFIED_SOURCE`] is only a don't-care while it cannot equal
/// an id the registry assigns. If allocation ever starts elsewhere or
/// reuses ids, the write intents in this file silently become
/// same-processor stores and the reservation sweeps stop running.
#[test]
fn the_unspecified_source_is_not_an_id_the_registry_assigns() {
    let mut units = UnitRegistry::new();
    // Comfortably above the handful any test here registers.
    for _ in 0..64 {
        let id = units.register_with(DummyUnit::runnable);
        assert_ne!(
            id, UNSPECIFIED_SOURCE,
            "the registry assigned the id write_intent uses as its don't-care source",
        );
    }
}

#[test]
fn same_unit_acquire_then_own_store_preserves_reservation() {
    // Acquire at effect index 0, write at effect index 1 from the
    // SAME unit; the write's clear sweep must preserve the
    // emitter's own entry. PPC spec only invalidates on stores from
    // *another* processor [PPC-Book2 p:10 s:1.7.3.1].
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![
            reservation_acquire(0x100, u),
            write_intent_from(0x140, vec![1, 2, 3, 4], u),
        ],
    );
    bed.process(&r, &e).unwrap();
    assert!(
        bed.reservations.is_held_by(u),
        "the write's source is {u:?}, the same unit that holds the reservation, \
         so the clear sweep must skip it",
    );
}

#[test]
fn other_unit_store_clears_reservation_on_same_line() {
    // Cross-unit store DOES invalidate: the write's source is a
    // different unit from the one holding the reservation, so the
    // clear sweep takes effect.
    let mut bed = CommitTestBed::new(4096);
    let holder = UnitId::new(99);
    let writer = UnitId::new(7);
    assert_ne!(writer, holder, "the cross-unit path needs distinct ids");
    bed.reservations
        .insert_or_replace(holder, cellgov_sync::ReservedLine::containing(0x100));
    // Floor: without a reservation to clear, the post-condition below
    // holds for a table that was empty all along.
    assert!(
        bed.reservations.is_held_by(holder),
        "setup must seat a reservation for the sweep to clear",
    );
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![write_intent_from(0x140, vec![1, 2, 3, 4], writer)],
    );
    bed.process(&r, &e).unwrap();
    assert!(
        !bed.reservations.is_held_by(holder),
        "a store from {writer:?} must clear {holder:?}'s reservation on the same line",
    );
}

#[test]
fn conditional_store_payload_length_mismatch_rejects_batch() {
    let mut bed = CommitTestBed::new(4096);
    let u = UnitId::new(8);
    bed.reservations
        .insert_or_replace(u, cellgov_sync::ReservedLine::containing(0x100));
    // Range length 4 but payload is 2 bytes -> validation error.
    let bad = Effect::ConditionalStore {
        range: range(0x100, 4),
        bytes: WritePayload::new(vec![0, 0]),
        ordering: PriorityClass::Normal,
        source: u,
        source_time: GuestTicks::new(0),
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![bad]);
    let err = bed.process(&r, &e);
    assert!(matches!(
        err,
        Err(CommitError::PayloadLengthMismatch { effect_index: 0 })
    ));
    // Reservation still present; batch aborted atomically.
    assert!(bed.reservations.is_held_by(u));
}

#[test]
fn fault_step_discards_reservation_effects() {
    let mut bed = CommitTestBed::new(4096);
    let u = UnitId::new(9);
    let (mut r, e) = step_with(
        YieldReason::Fault,
        vec![
            reservation_acquire(0x100, u),
            conditional_store(0x200, vec![1, 2, 3, 4], u),
        ],
    );
    r.fault = Some(FaultKind::Validation);
    let outcome = bed.process(&r, &e).unwrap();
    assert!(outcome.fault_discarded);
    // The fault path reports how many effects were dropped so
    // trace/replay tooling can correlate the outcome record with
    // the per-effect TraceRecord::EffectEmitted stream.
    assert_eq!(outcome.effects_discarded_on_fault, 2);
    // Nothing applied.
    assert!(bed.reservations.is_empty());
    assert!(bed
        .mem
        .read(range(0x200, 4))
        .unwrap()
        .iter()
        .all(|b| *b == 0));
}

#[test]
fn reservation_acquire_with_unregistered_source_rejects_batch() {
    let mut bed = CommitTestBed::new(4096);
    let ghost = UnitId::new(99); // never registered
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![reservation_acquire(0x100, ghost)],
    );
    let err = bed.process(&r, &e);
    assert!(matches!(
        err,
        Err(CommitError::UnknownSourceUnit {
            effect_index: 0,
            source_unit
        }) if source_unit == ghost
    ));
    // Nothing pollutes the table on rejection.
    assert!(bed.reservations.is_empty());
}

#[test]
fn conditional_store_with_unregistered_source_rejects_batch() {
    let mut bed = CommitTestBed::new(4096);
    let ghost = UnitId::new(99);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![conditional_store(0x100, vec![1, 2, 3, 4], ghost)],
    );
    let err = bed.process(&r, &e);
    assert!(matches!(
        err,
        Err(CommitError::UnknownSourceUnit {
            effect_index: 0,
            source_unit
        }) if source_unit == ghost
    ));
    // Store did not commit to memory on rejection.
    assert!(bed
        .mem
        .read(range(0x100, 4))
        .unwrap()
        .iter()
        .all(|b| *b == 0));
}

#[test]
fn conditional_store_without_prior_reservation_bumps_counter() {
    // Emitter-side LL/SC bug surface: a ConditionalStore reaches
    // apply with no reservation entry for the source. The pipeline
    // still commits the store (soft contract) but increments the
    // observability counter so real-emitter CI can assert it stays
    // zero across whole scenarios.
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    // Note: no reservation is inserted for `u` before the store.
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![conditional_store(0x100, vec![1, 2, 3, 4], u)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.conditional_stores_committed, 1);
    assert_eq!(outcome.conditional_stores_without_prior_reservation, 1);
    assert_eq!(outcome.reservations_cleared, 0);
    // Store still committed.
    let read = bed.mem.read(range(0x100, 4)).unwrap();
    assert_eq!(read, &[1, 2, 3, 4]);
}

#[test]
fn conditional_store_with_prior_reservation_leaves_counter_at_zero() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    bed.reservations
        .insert_or_replace(u, cellgov_sync::ReservedLine::containing(0x100));
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![conditional_store(0x100, vec![1, 2, 3, 4], u)],
    );
    let outcome = bed.process(&r, &e).unwrap();
    assert_eq!(outcome.conditional_stores_without_prior_reservation, 0);
    assert_eq!(outcome.reservations_cleared, 1);
}

#[test]
fn dma_enqueue_destination_out_of_range_rejects_batch() {
    let mut bed = CommitTestBed::new(4096);
    let issuer = bed.units.register_with(DummyUnit::runnable);
    // Source is in-range (0..4), destination is past end of memory.
    let bad_dst = ByteRange::new(GuestAddr::new(0x10_0000), 4).unwrap();
    let ok_src = range(0, 4);
    let req = DmaRequest::new(DmaDirection::Put, ok_src, bad_dst, issuer).unwrap();
    let bad = Effect::DmaEnqueue {
        request: req,
        payload: None,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![bad]);
    let err = bed.process(&r, &e);
    assert!(matches!(
        err,
        Err(CommitError::DmaDestinationOutOfRange { effect_index: 0 })
    ));
    // Queue stays empty when the batch is rejected atomically.
    assert!(bed.dma_queue.is_empty());
}

/// A guest that never reached GCM init has no label base, so the
/// offset it emits is the absolute address it wants written. The
/// RSX microtests under `tests/micro/` do exactly this.
#[test]
fn rsx_label_write_past_the_label_area_commits_when_no_label_base_exists() {
    let mut bed = CommitTestBed::new(0x2_0000);
    let effect = Effect::RsxLabelWrite {
        offset: 0x1_0000,
        value: 0xCAFE_BABE,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let outcome = bed.process(&r, &e).expect("absolute offset commits");
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        bed.mem.read(range(0x1_0000, 4)).unwrap(),
        &0xCAFE_BABEu32.to_be_bytes()
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "escapes the")]
fn rsx_label_write_past_the_label_area_trips_the_guard_under_a_label_base() {
    let mut bed = CommitTestBed::new(0x2_0000);
    bed.label_base = 0x2000;
    let effect = Effect::RsxLabelWrite {
        offset: cellgov_ps3_abi::lv2::rsx::reports::SIZE as u32,
        value: 1,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let _ = bed.process(&r, &e);
}

#[test]
fn rsx_label_write_inside_the_semaphore_region_commits_relative_to_the_label_base() {
    let mut bed = CommitTestBed::new(0x1_0000);
    bed.label_base = 0x2000;
    let effect = Effect::RsxLabelWrite {
        offset: 0x40,
        value: 0x1234_5678,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let outcome = bed.process(&r, &e).expect("relative offset commits");
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        bed.mem.read(range(0x2040, 4)).unwrap(),
        &0x1234_5678u32.to_be_bytes()
    );
}

/// The label base addresses the whole `RsxReports` area, so the report
/// entries at 0x1400 are a destination `NV4097_GET_REPORT` legitimately
/// names -- not a write past the end of the semaphore block.
#[test]
fn rsx_label_write_into_the_report_region_commits_without_tripping_the_guard() {
    let mut bed = CommitTestBed::new(0x1_0000);
    bed.label_base = 0x2000;
    let offset = cellgov_ps3_abi::lv2::rsx::driver_info_init::REPORTS_REPORT_OFFSET;
    let effect = Effect::RsxLabelWrite {
        offset,
        value: 0x0BAD_F00D,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let outcome = bed.process(&r, &e).expect("report offset commits");
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        bed.mem.read(range(0x2000 + offset as u64, 4)).unwrap(),
        &0x0BAD_F00Du32.to_be_bytes()
    );
}

#[test]
fn an_rsx_label_write_clears_a_reservation_it_lands_on() {
    let mut bed = CommitTestBed::new(0x1_0000);
    bed.label_base = 0x2000;
    let holder = UnitId::new(11);
    bed.reservations
        .insert_or_replace(holder, cellgov_sync::ReservedLine::containing(0x2040));
    assert!(
        bed.reservations.is_held_by(holder),
        "setup must seat a reservation for the sweep to clear",
    );
    let effect = Effect::RsxLabelWrite {
        offset: 0x40,
        value: 7,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let outcome = bed.process(&r, &e).expect("label write commits");
    assert_eq!(outcome.reservations_cleared, 1);
    assert!(
        !bed.reservations.is_held_by(holder),
        "the RSX is another bus master, so its store must clear {holder:?}'s reservation",
    );
}

#[test]
fn an_rsx_label_write_leaves_a_reservation_on_an_unrelated_line_alone() {
    let mut bed = CommitTestBed::new(0x1_0000);
    bed.label_base = 0x2000;
    let holder = UnitId::new(11);
    // A full line away from 0x2040, so the four bytes cannot overlap.
    bed.reservations
        .insert_or_replace(holder, cellgov_sync::ReservedLine::containing(0x3000));
    let effect = Effect::RsxLabelWrite {
        offset: 0x40,
        value: 7,
    };
    let (r, e) = step_with(YieldReason::BudgetExhausted, vec![effect]);
    let outcome = bed.process(&r, &e).expect("label write commits");
    assert_eq!(outcome.reservations_cleared, 0);
    assert!(bed.reservations.is_held_by(holder));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "the discard contract is driven by the yield reason")]
fn a_fault_effect_in_a_non_faulting_batch_trips_the_discard_guard() {
    let mut bed = CommitTestBed::new(4096);
    let u = bed.units.register_with(DummyUnit::runnable);
    let (r, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![Effect::FaultRaised {
            kind: FaultKind::Guest(0x300),
            source: u,
        }],
    );
    let _ = bed.process(&r, &e);
}
