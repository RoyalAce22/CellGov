//! Conservative dependency analysis for schedule exploration.
//!
//! [`StepFootprint`] holds the shared resources one step touched, and
//! [`StepFootprint::conflicts`] answers whether swapping two steps
//! could change what either observes. The answer over-approximates: it
//! never calls two dependent steps independent, and a false dependency
//! costs exploration budget alone.
//!
//! Each clause of `conflicts` carries the argument that makes it
//! sound. Some shared state reaches no footprint at all. Two steps
//! that interact only through one of these prune when they should
//! not:
//!
//! - Guest time, in two of its three readers. One clock advances by
//!   each step's cost. A `mftb` reads it into a guest register, and a
//!   timer wake is stamped with a deadline from it; neither reaches a
//!   footprint. `tests/shared_clock.rs` holds the witness. The third
//!   reader, the tick a transfer lands at, reaches a footprint through
//!   [`StepFootprint::inflight_dma_ranges`].
//! - The RSX FIFO advance pass, whose effects commit guest memory and
//!   sweep reservations from no unit's step.
//! - Every LV2 handler effect, guest write and wake alike: a handler
//!   commits through `Runtime::host_write`, not through the unit's
//!   step effects. No fake-ISA workload runs a handler.
//! - An LV2 syscall's park and the wake that ends it, neither of which
//!   carries an effect or a yield reason.
//! - `Runtime::set_unit_status_override`, which `cellgov_boot` uses
//!   mid-run to hold every other unit `Blocked` across a spawned
//!   child's `module_start`.
//!
//! A faulted step records no footprint and needs none: its batch is
//! discarded and every driver stops there, so it is not an event.
//!
//! Another unit's write can decide whether a step faults. An order
//! where the step does not fault records its read, and the relation
//! holds that pair apart. Where every order faults, the run truncates
//! and answers for nothing. `tests/fault_decided_by_a_write.rs` holds
//! both.

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;
use cellgov_mem::ByteRange;
use cellgov_sync::{BarrierId, MailboxId, SignalId, RESERVATION_LINE_BYTES};

/// Shared resources one execution step accessed.
///
/// Build via [`StepFootprint::from_step`], which reads the whole step.
/// [`StepFootprint::from_effects`] reads the effect list alone and
/// records no park the step result carries.
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
    /// Units this step's effects can park.
    ///
    /// The commit pipeline reads a wait's source, not its target, so
    /// the unit is the key a wake matches. A receive attempt belongs
    /// here too: it parks its source when the mailbox comes back empty.
    pub wait_units: Vec<cellgov_event::UnitId>,
    /// 128-byte-aligned line addresses touched by a `ReservationAcquire`.
    ///
    /// A cross-unit write overlapping the line clears the reservation
    /// and flips the next conditional-store verdict, so the pair
    /// conflicts.
    pub reservation_lines: Vec<u64>,
    /// Source and destination of every transfer in flight during this
    /// step.
    ///
    /// A transfer lands at the first commit whose clock passed the tick
    /// stamped at its enqueue. So a step taken during a flight moves
    /// the landing relative to every other step.
    ///
    /// A step taken before the enqueue needs no record. It moves the
    /// enqueue and everything after it by the same cost, which leaves
    /// the landing where it was relative to them. Reordering a step
    /// across the enqueue is still covered: a step during the flight
    /// conflicts with the enqueue's own `dma_ranges`.
    ///
    /// [`StepFootprint::conflicts`] tests this set against the other
    /// step's accesses, never against its own copy. Two steps that
    /// merely share a flight therefore still prune.
    pub inflight_dma_ranges: Vec<ByteRange>,
}

impl StepFootprint {
    /// Extract a footprint from one whole step.
    ///
    /// Adds the park a [`YieldReason::DmaWait`] result carries, which
    /// no effect names and [`StepFootprint::from_effects`] misses.
    pub fn from_step(
        unit: cellgov_event::UnitId,
        yielded: YieldReason,
        effects: &[Effect],
    ) -> Self {
        let mut fp = Self::from_effects(effects);
        if yielded.parks_without_an_effect() {
            fp.wait_units.push(unit);
        }
        fp
    }

    /// Widen every range category to the sibling views it aliases.
    ///
    /// An access through one view of a shared mapping reaches every
    /// sibling view's bytes. A DMA range needs the same widening: a
    /// transfer lands in space 0, and a unit that reads a view which
    /// aliases the landing sees it.
    ///
    /// Call it after [`StepFootprint::note_inflight`], so the
    /// in-flight set is there to widen.
    pub fn expand_aliases(&mut self, rt: &cellgov_core::Runtime, unit: cellgov_event::UnitId) {
        for category in [
            &mut self.shared_writes,
            &mut self.shared_reads,
            &mut self.dma_ranges,
            &mut self.inflight_dma_ranges,
        ] {
            let aliases: Vec<ByteRange> = category
                .iter()
                .flat_map(|range| rt.shared_alias_ranges(unit, *range))
                .collect();
            category.extend(aliases);
        }
    }

    /// Record the transfers in flight during this step.
    ///
    /// Call it after the step's commit: that commit is what fires a due
    /// transfer, and one it fired was in flight for that step.
    pub fn note_inflight(&mut self, rt: &cellgov_core::Runtime) {
        let queued = rt
            .dma_queue()
            .pending()
            .map(|c| (c.source(), c.destination()));
        let fired = rt
            .last_dma_completions()
            .iter()
            .map(|c| (c.source(), c.destination()));
        for (source, destination) in queued.chain(fired) {
            self.inflight_dma_ranges.push(source);
            self.inflight_dma_ranges.push(destination);
        }
    }

    /// Extract a footprint from the effects emitted in one step.
    ///
    /// `FaultRaised` and `TraceMarker` carry no guest state. The two
    /// RSX variants do, but they reach a batch from the FIFO advance
    /// pass rather than from a unit.
    ///
    /// [`StepFootprint::from_step`] adds the park.
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
                Effect::MailboxReceiveAttempt { mailbox, source } => {
                    fp.mailbox_receives.push(*mailbox);
                    // The footprint is built before the pop decides, so
                    // every attempt records the park it may take: an
                    // empty mailbox blocks the source, and only a wake
                    // naming it runs it again.
                    fp.wait_units.push(*source);
                }
                Effect::DmaEnqueue { request, .. } => {
                    fp.dma_ranges.push(request.source());
                    fp.dma_ranges.push(request.destination());
                }
                Effect::WaitOnEvent { target, source } => {
                    fp.wait_units.push(*source);
                    match target {
                        cellgov_effects::WaitTarget::Mailbox(id) => fp.wait_mailboxes.push(*id),
                        cellgov_effects::WaitTarget::Signal(id) => fp.wait_signals.push(*id),
                        cellgov_effects::WaitTarget::Barrier(id) => fp.wait_barriers.push(*id),
                    }
                }
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
    /// The two steps must belong to different units. The reservation
    /// and DMA clauses read only the cross-unit half of the hardware
    /// rule, so a same-unit pair gets an answer the hardware does not
    /// give. Program order already holds a unit's own steps apart.
    ///
    /// Returns `true` unless it can prove the pair independent.
    /// O(n*m) over each category's populated vectors, and a step
    /// usually populates one or two.
    pub fn conflicts(&self, other: &StepFootprint) -> bool {
        for a in &self.shared_writes {
            for b in &other.shared_writes {
                if a.overlaps(*b) {
                    return true;
                }
            }
        }

        // Bernstein's other two intersections.
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

        // A source range rides in `dma_ranges` beside its destination.
        // The destination carries the dependency: `apply_dma_transfer`
        // writes it at completion. The source is real only for a
        // payload-less transfer, which reads it at completion. An SPU
        // put copies local store into the payload at enqueue, so
        // pairing on its source is a false dependency.
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

        // Two receive attempts on one mailbox are order-dependent with
        // no step sending: the pipeline pops the FIFO for whichever
        // commits first and blocks the other, so the swap decides which
        // unit takes the message.
        if ids_overlap(&self.mailbox_receives, &other.mailbox_receives) {
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

        // A wake enables only the unit it names: `WaitOnEvent` blocks
        // its source and reads no target, and the one path back to
        // runnable is a `WakeUnit` naming that unit. An empty receive
        // parks its source the same way, and `wait_units` carries it.
        //
        // Two wakes belong to no step and reach no footprint: a DMA
        // completion waking its issuer, and a timer wake.
        if ids_overlap(&self.wake_targets, &other.wait_units)
            || ids_overlap(&other.wake_targets, &self.wait_units)
        {
            return true;
        }

        // No barrier releases anything. `WaitOnEvent` names only
        // `source` and blocks it, and no registry holds barrier state,
        // so a wait's only consequence is its own unit's status. The
        // same-barrier clause below is therefore a false dependency.
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
        // reservation whose 128-byte line its destination touches. That
        // clear flips the next conditional store's verdict, even where
        // the transferred bytes miss the store's own range. The sweep
        // in `Runtime::host_write` exempts the issuer alone. The source
        // half of `dma_ranges` rides along and over-approximates.
        if write_covers_any_line(&self.dma_ranges, &other.reservation_lines)
            || write_covers_any_line(&other.dma_ranges, &self.reservation_lines)
        {
            return true;
        }

        if lines_overlap(&self.reservation_lines, &other.reservation_lines) {
            return true;
        }

        // One step's cost decides where an in-flight transfer lands,
        // and the other step reads or writes the bytes it lands on.
        if touches_inflight(self, other) || touches_inflight(other, self) {
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
            && self.wait_units.is_empty()
            && self.wake_targets.is_empty()
            && self.reservation_lines.is_empty()
            && self.inflight_dma_ranges.is_empty()
    }
}

/// True when a transfer was in flight during `during`'s step, and
/// `accessor`'s step touched the bytes that transfer moves.
///
/// The last clause reads the landing as the write it is, the way the
/// `dma_ranges` clause above does: the destination sweeps every other
/// unit's reservation whose line it covers, so where it lands decides
/// whether the holder keeps the entry.
fn touches_inflight(during: &StepFootprint, accessor: &StepFootprint) -> bool {
    ranges_overlap(&during.inflight_dma_ranges, &accessor.shared_writes)
        || ranges_overlap(&during.inflight_dma_ranges, &accessor.shared_reads)
        || ranges_overlap(&during.inflight_dma_ranges, &accessor.dma_ranges)
        || write_covers_any_line(&during.inflight_dma_ranges, &accessor.reservation_lines)
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

/// Whether any of `writes` touches a byte of any 128-byte line in
/// `lines`.
///
/// Neither `saturating_add` can saturate, so neither can fold a
/// wrapped range onto the top of the address space and hide the low
/// bytes it would touch:
///
/// - `ByteRange::new` refuses a range whose exclusive end overflows,
///   and `ByteRange::contiguous_u32` cannot build one, so
///   `w_start + (w_len - 1)` stays below `u64::MAX`.
/// - `from_effects` masks every line address to a multiple of the
///   granule, so `line_addr + 127` reaches `u64::MAX` at the top line
///   and no further. The loop debug-asserts that mask.
fn write_covers_any_line(writes: &[ByteRange], lines: &[u64]) -> bool {
    for w in writes {
        let w_start = w.start().raw();
        let w_len = w.length();
        if w_len == 0 {
            continue;
        }
        let w_end = w_start.saturating_add(w_len - 1);
        for &line_addr in lines {
            // `from_effects` masks every line it pushes, and
            // `reservation_lines` is public, so a caller can push an
            // unmasked address.
            debug_assert_eq!(
                line_addr & (RESERVATION_LINE_BYTES - 1),
                0,
                "reservation line {line_addr:#x} is not granule-aligned",
            );
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
