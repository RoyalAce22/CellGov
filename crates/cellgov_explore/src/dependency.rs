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
//! - Guest time, in one of its four readers. One clock advances by
//!   each step's cost, and a timer wake is stamped with a deadline from
//!   it; that stamp reaches no footprint. `tests/timer_deadline.rs`
//!   holds the witness, and records that the cover stays whole anyway.
//!   The other three readers do reach a footprint. A `mftb` and an LV2
//!   handler's tick-derived write reach [`StepFootprint::reads_clock`].
//!   The tick a transfer lands at reaches
//!   [`StepFootprint::inflight_dma_ranges`].
//! - The RSX FIFO advance pass's cursor and call stack, and the label,
//!   notify and report words it writes. Those land as `RsxLabelWrite`
//!   effects the pass queues for the next commit, and
//!   `Runtime::commit_step` prepends them. No driver's effect slice
//!   carries one, and [`StepFootprint::from_effects`] records nothing
//!   for the variant either. The pass's MMIO mirrors are the half that
//!   does reach a footprint, through
//!   [`StepFootprint::note_host_writes`].
//! - An LV2 syscall's park and the wake that ends it, neither of which
//!   carries an effect or a yield reason.
//! - Whatever the all-blocked time warp does before it picks a step.
//!   The warp fires the timer wakes and the sync wakes itself, so a
//!   timed wait's expiry lands its repair writes and its wake
//!   continuations there. The `commit_step` that follows clears the
//!   records that hold them, before any footprint reads them. The DMA
//!   half of the warp is the one part covered.
//!   [`StepFootprint::inflight_dma_ranges`] holds a transfer's ranges
//!   for every step of its flight, rather than for the commit that
//!   fires it.
//!
//! An LV2 handler's own effects are not among them. The runtime applies
//! them at dispatch rather than through the commit pipeline, so the
//! calling unit's step names none of them.
//! [`StepFootprint::note_lv2_effects`] puts them back:
//!
//! - A write becomes that step's write.
//! - A mailbox send becomes that step's send, and the wake it performs
//!   on the unit whose id keys the mailbox.
//!
//! `tests/lv2_out_param.rs` and `tests/lv2_mailbox_wake.rs` hold the
//! two halves.
//!
//! The park reaches a workload here through one syscall, the spawn in
//! `tests/child_init_window.rs`. A refusal closes it, rather than a
//! record. `cellgov_boot` holds every other unit
//! `Blocked` across a spawned child's `module_start` through
//! `Runtime::set_unit_status_override`, which no footprint sees either.
//! So every driver stops with
//! [`crate::util::StopReason::ChildInitUnserved`] before it takes a
//! step under a staged pass, and the step that stages one reaches no
//! decision point. A window that spans one reaches no verdict, which is
//! the honest answer while the relation cannot see the parks.
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
    /// Ranges a transfer writes at completion: its destination.
    ///
    /// The commit pipeline refuses any direction but a put, and a put's
    /// destination is the end that lands in committed memory. So this
    /// end is the destination for everything that reaches a queue.
    pub dma_writes: Vec<ByteRange>,
    /// Ranges a transfer reads at completion: the source of a
    /// transfer no inline payload carries.
    ///
    /// A payloaded transfer copied its bytes at enqueue and reads
    /// nothing later, and an SPU put's source addresses local store
    /// rather than committed memory. Neither belongs here, so no
    /// comparison pairs a local-store address against a
    /// main-memory range.
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
    /// What every transfer in flight during this step touches at its
    /// landing: each one's destination, and the source of one no inline
    /// payload carries.
    ///
    /// A transfer lands at the first commit whose clock passed the tick
    /// stamped at its enqueue. So a step taken during a flight moves
    /// the landing relative to every other step.
    ///
    /// A step taken before the enqueue needs no record. It moves the
    /// enqueue and everything after it by the same cost, which leaves
    /// the landing where it was relative to them. Reordering a step
    /// across the enqueue is still covered: a step during the flight
    /// conflicts with the enqueue's own [`StepFootprint::dma_writes`]
    /// and [`StepFootprint::dma_reads`].
    ///
    /// [`StepFootprint::conflicts`] tests this set against the other
    /// step's accesses, and against this step's own accesses through
    /// the landing clause. Two steps that only share a flight still
    /// prune, where neither touches the bytes it moves.
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
    /// Call it after the step's commit: that commit is what fires a due
    /// transfer, and one it fired was in flight for that step.
    ///
    /// A transfer the all-blocked time warp fires reaches neither the
    /// queue nor the fired list, and needs to reach neither: the warp
    /// lands it before the step it then selects, and every step it was
    /// in flight for already read it out of the queue.
    pub fn note_inflight(&mut self, rt: &cellgov_core::Runtime) {
        let queued = rt.dma_queue().pending();
        let fired = rt
            .last_dma_completions()
            .iter()
            .map(|(completion, payloaded)| (completion, *payloaded));
        for (completion, payloaded) in queued.chain(fired) {
            self.inflight_dma_ranges.push(completion.destination());
            // A payloaded transfer reads no source at completion, and
            // an SPU put's source names local store. See
            // [`StepFootprint::dma_reads`].
            if !payloaded {
                self.inflight_dma_ranges.push(completion.source());
            }
        }
    }

    /// Record what an LV2 handler did during this step.
    ///
    /// Call it after the step's commit, which is what runs the
    /// dispatch. The runtime applies a handler's effects there rather
    /// than through the commit pipeline, so the step's own effect list
    /// names none of them. A syscall reaches the relation as a step
    /// that touched nothing.
    ///
    /// It reads them through [`StepFootprint::from_effects`], so each
    /// one lands in the category it would have from a unit and answers
    /// to the clause that category already has.
    ///
    /// A handler's mailbox send answers to one more. The commit
    /// pipeline's own send only fills the mailbox.
    /// `Runtime::apply_lv2_effects` runs a handler's send itself and
    /// releases the target's park with it, and it reads the target's
    /// status alone rather than what parked it. The mailbox clause
    /// pairs that send with a receive attempt and with nothing else, so
    /// this method records the release as the wake it is.
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
    /// Call it after the step's commit, before
    /// [`StepFootprint::expand_aliases`]. The commit runs the LV2
    /// dispatch, resolves the wakes and fires the completions, and each
    /// of those writes guest memory outside the unit's batch.
    ///
    /// Each one becomes this step's own
    /// [`StepFootprint::shared_writes`]: the bytes land at this step,
    /// and this step's position decides them. The match is exhaustive,
    /// so a new `HostWriter` variant does not compile until this
    /// classifies it.
    pub fn note_host_writes(&mut self, rt: &cellgov_core::Runtime) {
        for (writer, range) in rt.last_host_writes() {
            match writer {
                // The dispatch holds the tick it ran at, and nothing in
                // the effect separates a payload built from that tick
                // from one that ignored it. The
                // `sys_time_get_current_time` out parameters are such a
                // payload. So an LV2 write reads the clock, on the same
                // argument [`StepFootprint::reads_clock`] makes for
                // `mftb`: the handler can put that value anywhere in
                // the bytes it lands.
                cellgov_trace::HostWriter::Lv2Effect
                | cellgov_trace::HostWriter::SyscallOutParam
                | cellgov_trace::HostWriter::WakeContinuation => {
                    self.shared_writes.push(*range);
                    self.reads_clock = true;
                }
                // The seed copy a newly attached view receives. Its
                // bytes are the segment's, so no tick reaches them.
                cellgov_trace::HostWriter::SharedViewSeed
                // The RSX mirrors, whose bytes are the cursor's and the
                // flip model's, and no tick's. The advance pass belongs
                // to no unit's step, but its writes land inside one
                // commit. That step's position decides when a poller
                // sees them.
                | cellgov_trace::HostWriter::RsxMirror => {
                    self.shared_writes.push(*range);
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
                    // every attempt records the park it may take: an
                    // empty mailbox blocks the source, and only a wake
                    // naming it runs it again.
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
        // in `Runtime::host_write` exempts the issuer alone, and reaches
        // the write's own range alone. So only the destination sweeps: a
        // completion's source read clears nothing.
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
    /// rides a landing conflicts with every step. No production caller
    /// reads it; the tests use it to state a footprint's shape.
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
    /// The ticks before this step decide where that landing falls.
    /// Every step carries ticks, so any other step's position against
    /// this one decides what this step sees. The pair conflicts whether
    /// or not the other step is in the flight itself.
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
/// The last clause reads the landing as the write it is, the way the
/// [`StepFootprint::dma_writes`] clause above does: the destination
/// sweeps every other unit's reservation whose line it covers, so where
/// it lands decides whether the holder keeps the entry. The in-flight
/// set over-approximates there, because it holds a payload-less
/// transfer's source beside its destination and a source read sweeps
/// nothing.
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
