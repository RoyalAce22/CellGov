//! Conservative dependency analysis for schedule exploration.
//!
//! [`StepFootprint`] holds the shared resources one step touched, and
//! [`StepFootprint::conflicts`] answers whether a swap of two steps
//! could change what either observes. The answer over-approximates
//! [FlanaganGodefroid2005 p:3 s:2.2]: a false dependency costs
//! exploration budget alone. A faulted step
//! records no footprint: the commit discards its batch and every driver
//! stops there, so it is not an event. An order in which the step does
//! not fault records its read instead
//! (`tests/fault_decided_by_a_write.rs`).
//!
//! Four kinds of shared state reach no footprint, so two steps that
//! interact only through one of them prune when they should not:
//!
//! - The deadline a timer wake carries (`tests/timer_deadline.rs`,
//!   which records that the cover stays whole anyway). The clock's
//!   other readers reach [`StepFootprint::reads_clock`] and
//!   [`StepFootprint::inflight_dma_ranges`].
//! - The RSX FIFO advance pass's cursor and call stack, and the
//!   `RsxLabelWrite` effects it queues. Its MMIO mirrors do reach
//!   [`StepFootprint::note_host_writes`].
//! - A DMA completion's wake of its issuer, which the commit applies
//!   as a status override and no `WakeUnit` effect carries. The park
//!   it ends does reach [`StepFootprint::wait_units`].
//! - An LV2 handler's park of its caller and the wake that ends it.
//!   The one park a workload here reaches is the spawn's staged pass
//!   (`tests/child_init_window.rs`), which every driver refuses with
//!   [`crate::util::StopReason::ChildInitUnserved`]; the parks the
//!   sync and timer handlers take reach no footprint and no refusal.

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
    /// Ranges a transfer writes at completion: its destination.
    ///
    /// The commit pipeline admits puts alone, so the destination is the
    /// end that lands in committed memory for every queued transfer.
    pub dma_writes: Vec<ByteRange>,
    /// Ranges a transfer reads at completion: the source of a
    /// transfer no inline payload carries.
    ///
    /// A payloaded transfer copied its bytes at enqueue, and an SPU
    /// put's source is a local-store address. Neither is a main-memory
    /// range, so neither belongs here.
    pub dma_reads: Vec<ByteRange>,
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
    /// A wake names the parked unit, so the pipeline keys a wait by its
    /// source. A receive attempt belongs here too: an empty mailbox
    /// parks its source.
    pub wait_units: Vec<cellgov_event::UnitId>,
    /// 128-byte-aligned line addresses touched by a `ReservationAcquire`.
    ///
    /// A cross-unit write overlapping the line clears the reservation
    /// and flips the next conditional-store verdict, so the pair
    /// conflicts.
    pub reservation_lines: Vec<u64>,
    /// What every transfer in flight during this step touches at its
    /// landing: each one's destination, and the source of one no inline
    /// payload carries.
    ///
    /// A transfer lands at the first commit whose clock passed its
    /// enqueue tick, so a step taken during a flight moves the landing
    /// relative to every other step. A step taken before the enqueue
    /// moves the enqueue and the landing alike, and needs no record.
    pub inflight_dma_ranges: Vec<ByteRange>,
    /// Whether the step read the guest clock.
    ///
    /// One clock advances by each step's cost, so the ticks every other
    /// unit spends decide the value this step read. A reader therefore
    /// conflicts with every step, whatever either of them touched.
    pub reads_clock: bool,
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

    /// Add what the step's commit did to the footprint its effects gave.
    ///
    /// Call it directly after `commit_step`, with the unit that ran.
    /// The order inside is fixed: [`StepFootprint::expand_aliases`]
    /// widens the ranges already recorded and no others, and
    /// [`StepFootprint::note_host_writes`] widens its own ranges through
    /// the space each landed in, which need not be the stepping unit's
    /// (see `Runtime::last_host_writes`).
    pub fn note_commit(&mut self, rt: &cellgov_core::Runtime, unit: cellgov_event::UnitId) {
        self.note_inflight(rt);
        self.note_lv2_effects(rt);
        self.expand_aliases(rt, unit);
        self.note_host_writes(rt);
    }

    /// Widen every range category to the sibling views it aliases.
    ///
    /// An access through one view of a shared mapping reaches every
    /// sibling view's bytes, and a unit that reads a view which aliases
    /// a DMA landing sees the landing. A range recorded after this call
    /// stays unwidened; [`StepFootprint::note_commit`] fixes the order.
    pub fn expand_aliases(&mut self, rt: &cellgov_core::Runtime, unit: cellgov_event::UnitId) {
        for category in [
            &mut self.shared_writes,
            &mut self.shared_reads,
            &mut self.dma_writes,
            &mut self.dma_reads,
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
    /// Call it after the step's commit: that commit fires a due
    /// transfer, and one it fired was in flight for the step. A
    /// transfer the warp fires reaches neither list
    /// (`Runtime::last_dma_completions`) and needs to reach neither:
    /// every step it was in flight for already read it out of the queue.
    pub fn note_inflight(&mut self, rt: &cellgov_core::Runtime) {
        let queued = rt.dma_queue().pending();
        let fired = rt
            .last_dma_completions()
            .iter()
            .map(|(completion, payloaded)| (completion, *payloaded));
        for (completion, payloaded) in queued.chain(fired) {
            self.inflight_dma_ranges.push(completion.destination());
            // See the doc on `dma_reads`.
            if !payloaded {
                self.inflight_dma_ranges.push(completion.source());
            }
        }
    }

    /// Record what an LV2 handler did during this step.
    ///
    /// Call it after the step's commit, which runs the dispatch. The
    /// step's own effect list names none of a handler's effects
    /// (`Runtime::last_lv2_effects`), so without this a syscall reaches
    /// the relation as a step that touched nothing.
    ///
    /// `Runtime::apply_lv2_effects` releases the target's park with a
    /// handler's mailbox send, and the mailbox clause pairs a send with
    /// a receive attempt alone. So the send is also recorded as the
    /// wake it performs.
    pub fn note_lv2_effects(&mut self, rt: &cellgov_core::Runtime) {
        let lv2 = Self::from_effects(rt.last_lv2_effects());
        self.merge(lv2);
        for effect in rt.last_lv2_effects() {
            if let Effect::MailboxSend { mailbox, .. } = effect {
                // The target the release names: the unit whose raw id
                // is the mailbox's, whatever its block reason was.
                self.wake_targets
                    .push(cellgov_event::UnitId::new(mailbox.raw()));
            }
        }
    }

    /// The exhaustive destructure makes a new field a compile error here.
    fn merge(&mut self, other: Self) {
        let Self {
            shared_writes,
            shared_reads,
            mailbox_sends,
            mailbox_receives,
            dma_writes,
            dma_reads,
            signal_updates,
            wait_mailboxes,
            wait_signals,
            wait_barriers,
            wake_targets,
            wait_units,
            reservation_lines,
            inflight_dma_ranges,
            reads_clock,
        } = other;
        self.shared_writes.extend(shared_writes);
        self.shared_reads.extend(shared_reads);
        self.mailbox_sends.extend(mailbox_sends);
        self.mailbox_receives.extend(mailbox_receives);
        self.dma_writes.extend(dma_writes);
        self.dma_reads.extend(dma_reads);
        self.signal_updates.extend(signal_updates);
        self.wait_mailboxes.extend(wait_mailboxes);
        self.wait_signals.extend(wait_signals);
        self.wait_barriers.extend(wait_barriers);
        self.wake_targets.extend(wake_targets);
        self.wait_units.extend(wait_units);
        self.reservation_lines.extend(reservation_lines);
        self.inflight_dma_ranges.extend(inflight_dma_ranges);
        self.reads_clock |= reads_clock;
    }

    /// Record the host writes that landed during this step.
    ///
    /// Call it after the step's commit. Each write becomes this step's
    /// own [`StepFootprint::shared_writes`], widened through the space
    /// it landed in: the bytes land at this step, and this step's
    /// position decides them. The record reaches back to the step's
    /// start, so it also holds what the warp wrote.
    pub fn note_host_writes(&mut self, rt: &cellgov_core::Runtime) {
        for (writer, space, range) in rt.last_host_writes() {
            let record = |fp: &mut Self, range: &cellgov_mem::ByteRange| {
                fp.shared_writes.push(*range);
                fp.shared_writes
                    .extend(rt.shared_alias_ranges_in(*space, *range));
            };
            match writer {
                // The dispatch holds the tick it ran at, and a handler
                // can land a tick-derived value anywhere in these bytes
                // (the `sys_time_get_current_time` out parameters are
                // one). So an LV2 write reads the clock, on the argument
                // [`StepFootprint::reads_clock`] makes for `mftb`.
                cellgov_trace::HostWriter::Lv2Effect
                | cellgov_trace::HostWriter::SyscallOutParam
                | cellgov_trace::HostWriter::WakeContinuation => {
                    record(self, range);
                    self.reads_clock = true;
                }
                // A new view's seed copy carries the segment's bytes,
                // and the RSX mirrors carry the cursor's and the flip
                // model's; no tick reaches either. The advance pass
                // belongs to no unit's step, but its writes land inside
                // one commit, whose step's position decides when a
                // poller sees them.
                cellgov_trace::HostWriter::SharedViewSeed
                | cellgov_trace::HostWriter::RsxMirror => {
                    record(self, range);
                }
                // A landing is already the in-flight set's, which holds
                // it for every step of the flight rather than for the
                // one commit that fired it.
                cellgov_trace::HostWriter::DmaCompletion => {}
                // `expand_aliases` widens each access to its sibling
                // views, so a fanout's target range is already paired
                // against whatever reads or writes it.
                cellgov_trace::HostWriter::SharedViewFanout => {}
                // The program driving the runtime, on its own account.
                // It makes no placement inside a step.
                cellgov_trace::HostWriter::Placement => {}
            }
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
                Effect::ClockRead { .. } => {
                    fp.reads_clock = true;
                }
                Effect::MailboxSend { mailbox, .. } => {
                    fp.mailbox_sends.push(*mailbox);
                }
                Effect::MailboxReceiveAttempt { mailbox, source } => {
                    fp.mailbox_receives.push(*mailbox);
                    // The footprint is built before the pop decides, so
                    // every attempt records the park it may take.
                    fp.wait_units.push(*source);
                }
                Effect::DmaEnqueue { request, payload } => {
                    fp.dma_writes.push(request.destination());
                    if payload.is_none() {
                        fp.dma_reads.push(request.source());
                    }
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
    /// outcome [FlanaganGodefroid2005 p:3 s:2.2 Definition 1].
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

        // Bernstein again, over what a transfer does at completion
        // against what the other step does now. Two reads are the one
        // pairing left out, here as above.
        if ranges_overlap(&self.shared_writes, &other.dma_writes)
            || ranges_overlap(&other.shared_writes, &self.dma_writes)
            || ranges_overlap(&self.shared_writes, &other.dma_reads)
            || ranges_overlap(&other.shared_writes, &self.dma_reads)
            || ranges_overlap(&self.shared_reads, &other.dma_writes)
            || ranges_overlap(&other.shared_reads, &self.dma_writes)
        {
            return true;
        }

        if ranges_overlap(&self.dma_writes, &other.dma_writes)
            || ranges_overlap(&self.dma_writes, &other.dma_reads)
            || ranges_overlap(&other.dma_writes, &self.dma_reads)
        {
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
        // runnable is a `WakeUnit` naming that unit.
        if ids_overlap(&self.wake_targets, &other.wait_units)
            || ids_overlap(&other.wake_targets, &self.wait_units)
        {
            return true;
        }

        // A false dependency: no registry holds barrier state, so a
        // barrier wait's one consequence is its own unit's status.
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

        // A landing clears every other unit's reservation whose line its
        // destination touches, even where the bytes miss the store's own
        // range (`Runtime::host_write` sweeps the write's range and
        // exempts the issuer alone). A source read clears nothing.
        if write_covers_any_line(&self.dma_writes, &other.reservation_lines)
            || write_covers_any_line(&other.dma_writes, &self.reservation_lines)
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

        // Every step carries ticks, so every step decides which side of
        // a landing the step that rides it falls on.
        if self.rides_a_landing() || other.rides_a_landing() {
            return true;
        }

        // The same ticks decide the value a clock reader saw, and it
        // can store that value anywhere.
        if self.reads_clock || other.reads_clock {
            return true;
        }

        false
    }

    /// True when the step accessed no shared resources.
    ///
    /// A step this returns `true` for can still conflict: one that
    /// rides a landing conflicts with every step.
    pub fn is_local_only(&self) -> bool {
        self.shared_writes.is_empty()
            && self.shared_reads.is_empty()
            && self.mailbox_sends.is_empty()
            && self.mailbox_receives.is_empty()
            && self.dma_writes.is_empty()
            && self.dma_reads.is_empty()
            && self.signal_updates.is_empty()
            && self.wait_mailboxes.is_empty()
            && self.wait_signals.is_empty()
            && self.wait_barriers.is_empty()
            && self.wait_units.is_empty()
            && self.wake_targets.is_empty()
            && self.reservation_lines.is_empty()
            && self.inflight_dma_ranges.is_empty()
            && !self.reads_clock
    }

    /// True when this step touches the bytes of a transfer that was in
    /// flight while it ran.
    ///
    /// Every step carries ticks, so any other step's position decides
    /// what this one sees, whether or not that step is in the flight.
    fn rides_a_landing(&self) -> bool {
        // `note_inflight` reads the queue after the commit that filed
        // the request, so an enqueue's destination always sits in its
        // own in-flight set. The step's own DMA ranges stay out of the
        // pairing for that reason.
        ranges_overlap(&self.inflight_dma_ranges, &self.shared_writes)
            || ranges_overlap(&self.inflight_dma_ranges, &self.shared_reads)
            || write_covers_any_line(&self.inflight_dma_ranges, &self.reservation_lines)
    }
}

/// True when a transfer was in flight during `during`'s step, and
/// `accessor`'s step touched the bytes that transfer moves.
///
/// The last clause reads a landing as the write it is, the way the
/// reservation clause of [`StepFootprint::conflicts`] does. It
/// over-approximates: the in-flight set holds a source beside its
/// destination, and a source read sweeps nothing.
fn touches_inflight(during: &StepFootprint, accessor: &StepFootprint) -> bool {
    ranges_overlap(&during.inflight_dma_ranges, &accessor.shared_writes)
        || ranges_overlap(&during.inflight_dma_ranges, &accessor.shared_reads)
        || ranges_overlap(&during.inflight_dma_ranges, &accessor.dma_writes)
        || ranges_overlap(&during.inflight_dma_ranges, &accessor.dma_reads)
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
/// Neither `saturating_add` can saturate, so neither folds a wrapped
/// range onto the top of the address space:
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
