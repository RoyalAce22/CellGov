//! Conservative dependency analysis for schedule exploration.
//!
//! [`StepFootprint`] summarizes one step's shared-resource accesses.
//! Two footprints conflict if swapping their execution order could
//! produce a different observable outcome; non-conflicting steps are
//! independent and the swap need not be explored.
//!
//! The analysis over-approximates dependency so that false
//! independencies never occur; false dependencies only waste
//! exploration budget.

use cellgov_effects::Effect;
use cellgov_mem::ByteRange;
use cellgov_sync::{BarrierId, MailboxId, SignalId, RESERVATION_LINE_BYTES};

/// Shared resources one execution step accessed.
///
/// Build via [`StepFootprint::from_effects`] from the step's emitted
/// effect list.
#[derive(Debug, Clone, Default)]
pub struct StepFootprint {
    /// Byte ranges written via `SharedWriteIntent` or `ConditionalStore`.
    pub shared_writes: Vec<ByteRange>,
    /// Mailboxes sent to.
    pub mailbox_sends: Vec<MailboxId>,
    /// Mailboxes read from.
    pub mailbox_receives: Vec<MailboxId>,
    /// DMA source and destination ranges (both appended).
    pub dma_ranges: Vec<ByteRange>,
    /// Signals updated.
    pub signal_updates: Vec<SignalId>,
    /// Mailbox wait targets.
    pub wait_mailboxes: Vec<MailboxId>,
    /// Signal wait targets.
    pub wait_signals: Vec<SignalId>,
    /// Barrier wait targets.
    pub wait_barriers: Vec<BarrierId>,
    /// Units explicitly woken.
    pub wake_targets: Vec<cellgov_event::UnitId>,
    /// 128-byte-aligned line addresses touched by a `ReservationAcquire`.
    ///
    /// A cross-unit write overlapping the line clears the reservation
    /// and flips the next conditional-store verdict, so the pair
    /// conflicts.
    pub reservation_lines: Vec<u64>,
}

impl StepFootprint {
    /// Extract a footprint from the effects emitted in one step.
    ///
    /// `FaultRaised` discards the whole step's effects upstream, and
    /// `TraceMarker` / RSX completion effects have no dependency
    /// impact, so all four are dropped.
    pub fn from_effects(effects: &[Effect]) -> Self {
        let mut fp = Self::default();
        for effect in effects {
            match effect {
                Effect::SharedWriteIntent { range, .. } => {
                    fp.shared_writes.push(*range);
                }
                Effect::MailboxSend { mailbox, .. } => {
                    fp.mailbox_sends.push(*mailbox);
                }
                Effect::MailboxReceiveAttempt { mailbox, .. } => {
                    fp.mailbox_receives.push(*mailbox);
                }
                Effect::DmaEnqueue { request, .. } => {
                    fp.dma_ranges.push(request.source());
                    fp.dma_ranges.push(request.destination());
                }
                Effect::WaitOnEvent { target, .. } => match target {
                    cellgov_effects::WaitTarget::Mailbox(id) => fp.wait_mailboxes.push(*id),
                    cellgov_effects::WaitTarget::Signal(id) => fp.wait_signals.push(*id),
                    cellgov_effects::WaitTarget::Barrier(id) => fp.wait_barriers.push(*id),
                },
                Effect::WakeUnit { target, .. } => {
                    fp.wake_targets.push(*target);
                }
                Effect::SignalUpdate { signal, .. } => {
                    fp.signal_updates.push(*signal);
                }
                Effect::ReservationAcquire { line_addr, .. } => {
                    fp.reservation_lines
                        .push(*line_addr & !(RESERVATION_LINE_BYTES - 1));
                }
                Effect::ConditionalStore { range, .. } => {
                    fp.shared_writes.push(*range);
                }
                Effect::FaultRaised { .. }
                | Effect::TraceMarker { .. }
                | Effect::RsxLabelWrite { .. }
                | Effect::RsxFlipRequest { .. } => {}
            }
        }
        fp
    }

    /// True when swapping these two steps could change the observable
    /// outcome.
    ///
    /// Returns `true` unless independence can be proved. O(n*m) in the
    /// product of each category's populated vectors; in practice a step
    /// touches only one or two categories so the cost is small.
    pub fn conflicts(&self, other: &StepFootprint) -> bool {
        for a in &self.shared_writes {
            for b in &other.shared_writes {
                if a.overlaps(*b) {
                    return true;
                }
            }
        }

        if ranges_overlap(&self.shared_writes, &other.dma_ranges)
            || ranges_overlap(&other.shared_writes, &self.dma_ranges)
        {
            return true;
        }

        if ranges_overlap(&self.dma_ranges, &other.dma_ranges) {
            return true;
        }

        if ids_overlap(&self.mailbox_sends, &other.mailbox_receives)
            || ids_overlap(&other.mailbox_sends, &self.mailbox_receives)
        {
            return true;
        }

        if ids_overlap(&self.mailbox_sends, &other.mailbox_sends) {
            return true;
        }

        if ids_overlap(&self.signal_updates, &other.signal_updates) {
            return true;
        }
        if ids_overlap(&self.signal_updates, &other.wait_signals)
            || ids_overlap(&other.signal_updates, &self.wait_signals)
        {
            return true;
        }

        // Any wake conflicts with any wait on the other side: wake
        // targets are often resolved indirectly through barriers or
        // mailboxes and tracking the exact pairing is not worth the
        // precision loss.
        if !self.wake_targets.is_empty() && other.has_any_wait() {
            return true;
        }
        if !other.wake_targets.is_empty() && self.has_any_wait() {
            return true;
        }

        if ids_overlap(&self.wait_barriers, &other.wait_barriers) {
            return true;
        }

        if write_covers_any_line(&self.shared_writes, &other.reservation_lines)
            || write_covers_any_line(&other.shared_writes, &self.reservation_lines)
        {
            return true;
        }

        if lines_overlap(&self.reservation_lines, &other.reservation_lines) {
            return true;
        }

        false
    }

    /// True when the step accessed no shared resources.
    pub fn is_local_only(&self) -> bool {
        self.shared_writes.is_empty()
            && self.mailbox_sends.is_empty()
            && self.mailbox_receives.is_empty()
            && self.dma_ranges.is_empty()
            && self.signal_updates.is_empty()
            && self.wait_mailboxes.is_empty()
            && self.wait_signals.is_empty()
            && self.wait_barriers.is_empty()
            && self.wake_targets.is_empty()
            && self.reservation_lines.is_empty()
    }

    /// Append every access from `other` into `self`.
    pub fn merge(&mut self, other: &StepFootprint) {
        self.shared_writes.extend_from_slice(&other.shared_writes);
        self.mailbox_sends.extend_from_slice(&other.mailbox_sends);
        self.mailbox_receives
            .extend_from_slice(&other.mailbox_receives);
        self.dma_ranges.extend_from_slice(&other.dma_ranges);
        self.signal_updates.extend_from_slice(&other.signal_updates);
        self.wait_mailboxes.extend_from_slice(&other.wait_mailboxes);
        self.wait_signals.extend_from_slice(&other.wait_signals);
        self.wait_barriers.extend_from_slice(&other.wait_barriers);
        self.wake_targets.extend_from_slice(&other.wake_targets);
        self.reservation_lines
            .extend_from_slice(&other.reservation_lines);
    }

    fn has_any_wait(&self) -> bool {
        !self.wait_mailboxes.is_empty()
            || !self.wait_signals.is_empty()
            || !self.wait_barriers.is_empty()
    }
}

fn ranges_overlap(a: &[ByteRange], b: &[ByteRange]) -> bool {
    for ra in a {
        for rb in b {
            if ra.overlaps(*rb) {
                return true;
            }
        }
    }
    false
}

fn ids_overlap<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    for x in a {
        for y in b {
            if x == y {
                return true;
            }
        }
    }
    false
}

fn write_covers_any_line(writes: &[ByteRange], lines: &[u64]) -> bool {
    for w in writes {
        let w_start = w.start().raw();
        let w_len = w.length();
        if w_len == 0 {
            continue;
        }
        let w_end = w_start.saturating_add(w_len - 1);
        for &line_addr in lines {
            let line_end = line_addr.saturating_add(RESERVATION_LINE_BYTES - 1);
            if w_start <= line_end && line_addr <= w_end {
                return true;
            }
        }
    }
    false
}

fn lines_overlap(a: &[u64], b: &[u64]) -> bool {
    for x in a {
        for y in b {
            if x == y {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_dma::{DmaDirection, DmaRequest};
    use cellgov_effects::payload::{MailboxMessage, WritePayload};
    use cellgov_event::{PriorityClass, UnitId};
    use cellgov_mem::GuestAddr;
    use cellgov_time::GuestTicks;

    fn range(start: u64, len: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), len).unwrap()
    }

    #[test]
    fn overlapping_writes_conflict() {
        let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
            range: range(0, 8),
            bytes: WritePayload::new(vec![0; 8]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(0),
            source_time: GuestTicks::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
            range: range(4, 8),
            bytes: WritePayload::new(vec![0; 8]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn disjoint_writes_are_independent() {
        let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
            range: range(0, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(0),
            source_time: GuestTicks::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
            range: range(8, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        }]);
        assert!(!a.conflicts(&b));
    }

    #[test]
    fn send_receive_same_mailbox_conflicts() {
        let a = StepFootprint::from_effects(&[Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(42),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
            mailbox: MailboxId::new(1),
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn send_receive_different_mailbox_independent() {
        let a = StepFootprint::from_effects(&[Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(42),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
            mailbox: MailboxId::new(2),
            source: UnitId::new(1),
        }]);
        assert!(!a.conflicts(&b));
    }

    #[test]
    fn two_sends_same_mailbox_conflict() {
        let a = StepFootprint::from_effects(&[Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(1),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(2),
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn signal_update_same_signal_conflicts() {
        let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0x1,
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0x2,
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn signal_update_vs_wait_conflicts() {
        let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0x1,
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: cellgov_effects::WaitTarget::Signal(SignalId::new(1)),
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn signal_update_different_signal_independent() {
        let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0x1,
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::SignalUpdate {
            signal: SignalId::new(2),
            value: 0x2,
            source: UnitId::new(1),
        }]);
        assert!(!a.conflicts(&b));
    }

    #[test]
    fn dma_overlapping_destination_conflicts() {
        let req_a = DmaRequest::new(
            DmaDirection::Put,
            range(0x100, 0x40),
            range(0x1000, 0x40),
            UnitId::new(0),
        )
        .unwrap();
        let req_b = DmaRequest::new(
            DmaDirection::Put,
            range(0x200, 0x40),
            range(0x1020, 0x40),
            UnitId::new(1),
        )
        .unwrap();
        let a = StepFootprint::from_effects(&[Effect::DmaEnqueue {
            request: req_a,
            payload: None,
        }]);
        let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
            request: req_b,
            payload: None,
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn write_vs_dma_overlapping_conflicts() {
        let req = DmaRequest::new(
            DmaDirection::Put,
            range(0x100, 0x40),
            range(0, 0x40),
            UnitId::new(1),
        )
        .unwrap();
        let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
            range: range(0x10, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(0),
            source_time: GuestTicks::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
            request: req,
            payload: None,
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn wake_vs_wait_conflicts() {
        let a = StepFootprint::from_effects(&[Effect::WakeUnit {
            target: UnitId::new(2),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn wait_vs_wake_conflicts_symmetric() {
        let a = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::WakeUnit {
            target: UnitId::new(3),
            source: UnitId::new(1),
        }]);
        assert!(
            a.conflicts(&b),
            "wait vs wake should conflict symmetrically"
        );
    }

    #[test]
    fn both_wait_same_barrier_conflicts() {
        let a = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(1)),
            source: UnitId::new(0),
        }]);
        let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(1)),
            source: UnitId::new(1),
        }]);
        assert!(a.conflicts(&b));
    }

    #[test]
    fn empty_footprints_are_independent() {
        let a = StepFootprint::default();
        let b = StepFootprint::default();
        assert!(!a.conflicts(&b));
    }

    #[test]
    fn trace_marker_only_is_local() {
        let fp = StepFootprint::from_effects(&[Effect::TraceMarker {
            marker: 0xCAFE,
            source: UnitId::new(0),
        }]);
        assert!(fp.is_local_only());
    }

    #[test]
    fn fault_is_ignored() {
        let fp = StepFootprint::from_effects(&[Effect::FaultRaised {
            kind: cellgov_effects::FaultKind::Validation,
            source: UnitId::new(0),
        }]);
        assert!(fp.is_local_only());
    }
}
