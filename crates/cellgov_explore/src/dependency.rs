//! Conservative dependency analysis for schedule exploration.
//!
//! [`StepFootprint`] summarizes one step's shared-resource accesses.
//! Two footprints conflict if swapping their execution order could
//! produce a different observable outcome; non-conflicting steps are
//! independent and the swap need not be explored. The analysis
//! over-approximates: it never reports two dependent steps as
//! independent, and a false dependency only costs exploration budget.
//!
//! A guest load of committed memory emits `Effect::SharedReadIntent`
//! for the bytes it read. The read set gives all three of Bernstein's
//! intersections over data accesses -- write-write, read-against-write
//! and write-against-read. A write-read race whose read steers a later
//! store to a disjoint address therefore conflicts here, and the
//! explorer covers it. Two loads still prune against each other, since
//! neither changes what the other observes.
//!
//! One access reaches committed memory and emits nothing: instruction
//! fetch. A unit that races another unit's write to the text region
//! prunes against it.

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
    /// Byte ranges read via `SharedReadIntent`.
    pub shared_reads: Vec<ByteRange>,
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
                Effect::SharedReadIntent { range, .. } => {
                    fp.shared_reads.push(*range);
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

        // Bernstein's other two intersections. No read-against-read
        // clause: neither read changes what the other sees.
        if ranges_overlap(&self.shared_reads, &other.shared_writes)
            || ranges_overlap(&other.shared_reads, &self.shared_writes)
        {
            return true;
        }

        if ranges_overlap(&self.shared_writes, &other.dma_ranges)
            || ranges_overlap(&other.shared_writes, &self.dma_ranges)
        {
            return true;
        }

        // A DMA's source range rides in `dma_ranges` alongside its
        // destination. Pairing reads against the whole vector
        // therefore adds read-against-read pairs for two units that
        // read one buffer.
        if ranges_overlap(&self.shared_reads, &other.dma_ranges)
            || ranges_overlap(&other.shared_reads, &self.dma_ranges)
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

        // No read-against-reservation clause: what loses a reservation
        // is a store or another modification of the granule, or an act
        // of the holder itself [PPC-Book2 p:10 s:1.7.3.1]. Another
        // unit's load leaves the entry, and the verdict of its later
        // conditional store, alone.
        if write_covers_any_line(&self.shared_writes, &other.reservation_lines)
            || write_covers_any_line(&other.shared_writes, &self.reservation_lines)
        {
            return true;
        }

        // A completed cross-unit DMA clears every other unit's
        // reservation whose 128-byte line its destination touches,
        // even when the transferred bytes miss the conditional
        // store's exact range (cellgov_core runtime/dma.rs,
        // `fire_dma_completions` -> `clear_covering`), flipping the
        // store's verdict. Source ranges ride along in `dma_ranges`;
        // pairing them too only over-approximates.
        if write_covers_any_line(&self.dma_ranges, &other.reservation_lines)
            || write_covers_any_line(&other.dma_ranges, &self.reservation_lines)
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
            && self.shared_reads.is_empty()
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
#[path = "tests/dependency_tests.rs"]
mod tests;
