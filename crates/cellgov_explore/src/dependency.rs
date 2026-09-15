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
//! Instruction fetch reads the text region too. A PPU block records
//! what it fetched at its boundary, coalesced into one read per run of
//! addresses and into one covering span past a cap on the runs, so a
//! unit that races another unit's write to that region conflicts with
//! it.
//!
//! One read of shared state still reaches no footprint: guest time.
//! One global clock advances per step. A DMA completion lands at the
//! first commit whose clock reached its completion tick, and a PPU
//! `mftb` reads that clock into a guest register. A step that touches
//! no shared resource still moves the clock relative to every later
//! step. Two steps this module calls independent can therefore commit
//! different memory when they swap. `tests/shared_clock.rs` holds the
//! witness.
//!
//! One write reaches no footprint either. The RSX FIFO advance pass
//! emits effects that commit guest memory and sweep reservations, and
//! `commit_step` prepends them to the next space-0 batch. Those
//! effects belong to no unit's step, so no footprint names them.
//!
//! Two ways a unit's status changes reach no footprint, so a wake of
//! that unit prunes against the step that parked it:
//!
//! - An LV2 `MailboxSend` handler returns a unit to runnable whatever
//!   parked it. A footprint reads the unit's own step effects. An LV2
//!   handler's effects commit through `Runtime::host_write` inside
//!   `commit_step` instead, and that same boundary hides
//!   LV2-committed guest writes. No workload built from the fake ISA
//!   reaches this path, because no LV2 handler runs.
//! - `Runtime::set_unit_status_override` is public, so the program
//!   driving the runtime can change a unit's status outside every
//!   path this module reasons about. `cellgov_boot` does, mid-run:
//!   `run_pending_child_inits` holds every other runnable unit
//!   `Blocked` across a spawned child's `module_start` and restores
//!   each one afterward. An exploration over a window of a boot that
//!   spawns a child covers those steps, so this one is reachable.
//!
//! A park the commit pipeline reads off the step result is visible:
//! [`StepFootprint::from_step`] reads it too, and
//! `YieldReason::parks_without_an_effect` is where a new yield reason
//! picks its side. One park stays invisible, and so does its wake:
//! LV2 dispatch parks a syscall's source with no effect and no yield
//! reason of its own, then returns it to runnable the same nameless
//! way.

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
    /// A wait names a mailbox, signal or barrier, and the commit
    /// pipeline reads none of them. It reads the source, so the unit
    /// is the key that matches a wake. A receive attempt parks its
    /// source the same way when the mailbox comes back empty, so its
    /// source belongs here too.
    pub wait_units: Vec<cellgov_event::UnitId>,
    /// 128-byte-aligned line addresses touched by a `ReservationAcquire`.
    ///
    /// A cross-unit write overlapping the line clears the reservation
    /// and flips the next conditional-store verdict, so the pair
    /// conflicts.
    pub reservation_lines: Vec<u64>,
}

impl StepFootprint {
    /// Extract a footprint from one whole step.
    ///
    /// The commit pipeline parks a unit that yields
    /// [`YieldReason::DmaWait`] from the step result alone, and no
    /// effect names that park. [`StepFootprint::from_effects`]
    /// therefore misses it, and a wake of the parked unit prunes
    /// against the step that parked it.
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

    /// Extract a footprint from the effects emitted in one step.
    ///
    /// `FaultRaised` discards the whole step's effects upstream and
    /// `TraceMarker` carries no guest state. The two RSX variants do
    /// commit guest state, but no execution unit emits them: they
    /// reach a batch from the FIFO advance pass.
    ///
    /// [`StepFootprint::from_step`] adds the park a step result
    /// carries.
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
                    // The commit pipeline blocks the source when
                    // `Mailbox::try_receive` comes back empty, and a
                    // later send wakes nobody. Only a wake naming the
                    // unit runs it again, so every attempt records the
                    // park it may take: the footprint is built before
                    // the pop decides.
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
    /// and DMA rules below read only the cross-unit half of the
    /// hardware rule. A same-unit pair therefore gets an answer the
    /// hardware does not give. Program order already holds a unit's
    /// own steps apart.
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
        // destination, so every pairing reads both halves. The
        // destination half carries the real dependency:
        // `apply_dma_transfer` writes it at completion. The source
        // half is real only for a transfer that carries no payload,
        // which reads its source at completion rather than at
        // enqueue. The SPU put path copies local store into the
        // payload at enqueue. Its source range addresses local store
        // rather than committed memory, so every pair that half adds
        // there is a false dependency.
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

        // Two receive attempts on one mailbox are order-dependent even
        // when no step sends. The commit pipeline's
        // `MailboxReceiveAttempt` arm pops the FIFO for whichever unit
        // commits first (`Mailbox::try_receive`), and blocks the other
        // when the pop returns empty. The swap therefore decides which
        // unit takes the message and which one parks.
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

        // A wake enables only the unit it names. The commit pipeline's
        // `WaitOnEvent` arm blocks its source and reads no target, and
        // the one path from there back to runnable is a `WakeUnit`
        // naming that unit, so a wake reaches no other unit's wait.
        // A receive attempt that finds the mailbox empty parks its
        // source the same way and `wait_units` carries that source
        // too, so the wake that runs it again pairs with it here.
        //
        // The two exceptions belong to no step and reach no footprint:
        // a DMA completion wakes its issuer, and a timer wake fires
        // from the runtime's own clock.
        if ids_overlap(&self.wake_targets, &other.wait_units)
            || ids_overlap(&other.wake_targets, &self.wait_units)
        {
            return true;
        }

        // Two units that wait on different barriers are independent
        // because no barrier releases anything. `WaitTarget` reaches
        // exactly one reader in the workspace, `from_effects` above.
        // The commit pipeline's `WaitOnEvent` arm names only `source`
        // and blocks it, and no registry holds barrier state. So a
        // wait's only guest-visible consequence is its own unit's
        // status, and one unit's wait can free no other unit's.
        //
        // The same-barrier clause below is therefore a false
        // dependency; it costs exploration budget alone.
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
        // reservation whose 128-byte line its destination touches. The
        // clear flips the store's verdict, even when the transferred
        // bytes miss the conditional store's own range.
        // `fire_dma_completions` hands the destination to
        // `Runtime::host_write`, which sweeps the reservation table
        // and exempts the issuer alone. The sweep asks each entry
        // whether its whole line overlaps the written bytes. The
        // source half of `dma_ranges` rides along and only
        // over-approximates.
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
            && self.wait_units.is_empty()
            && self.wake_targets.is_empty()
            && self.reservation_lines.is_empty()
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
