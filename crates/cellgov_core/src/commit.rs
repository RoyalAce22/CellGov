//! Commit pipeline -- pipeline steps 5-7 (validate, stage, apply).
//!
//! This module owns the journey from a single unit's
//! [`ExecutionStepResult`] to a mutation of [`GuestMemory`]. The
//! contract:
//!
//! - **One commit batch per unit yield.** No cross-unit
//!   batching. The commit applier is trivial and the trace is one-to-one
//!   with scheduling decisions.
//! - **Atomic from guest visibility.** Either every
//!   [`Effect::SharedWriteIntent`] in the batch becomes visible at the
//!   same epoch boundary, or none do.
//! - **Fault rule.** A step that yields with
//!   [`YieldReason::Fault`] commits nothing -- including effects that
//!   preceded the fault in emission order. The fault is recorded but no
//!   partial commit is permitted.
//! - **Validation rejection.** Validation may reject an entire batch
//!   (malformed effects, out-of-range writes). A rejected batch commits
//!   nothing and surfaces as a fault on the originating unit.
//!
//! The pipeline handles these effect types end to end:
//! `SharedWriteIntent` (memory commits), `MailboxSend` (FIFO push),
//! `MailboxReceiveAttempt` (pop or block), `SignalUpdate` (OR-merge),
//! `DmaEnqueue` (latency-modeled completion queue), `WakeUnit`
//! (status override to Runnable), `WaitOnEvent` (status override
//! to Blocked), `ReservationAcquire` (insert or replace a
//! per-unit entry in the atomic reservation table), and
//! `ConditionalStore` (apply the store, drop the emitter's
//! reservation entry, clear other units' entries covering the
//! line). Every committed `SharedWriteIntent` also runs the clear
//! sweep against the reservation table so a cross-unit store
//! invalidates any conflicting reservations. `FaultRaised` and
//! `TraceMarker` are counted but do not mutate runtime state.

use crate::registry::UnitRegistry;
use cellgov_dma::{DmaCompletion, DmaLatencyModel, DmaQueue};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus, YieldReason};
use cellgov_mem::{GuestMemory, MemError, StagedWrite, StagingMemory};
use cellgov_sync::{
    MailboxId, MailboxRegistry, ReservationTable, ReservedLine, SignalId, SignalRegistry,
};
use cellgov_time::GuestTicks;

/// Why a commit batch could not be applied.
///
/// Crate-local; there is no universal `Error` enum across the workspace. The
/// runtime layer maps these onto whatever fault reporting it uses
/// when it surfaces them to the originating unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitError {
    /// A `SharedWriteIntent`'s payload byte length did not match the
    /// length of its target range. The index identifies the offending
    /// effect's position in the original `emitted_effects` vector.
    PayloadLengthMismatch {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// A `SharedWriteIntent`'s target range is not entirely contained
    /// within a single registered memory region, or its end address
    /// overflows `u64`.
    OutOfRange {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// A `MailboxSend` referenced a `MailboxId` that is not registered
    /// in the runtime's mailbox registry. Aborts the entire batch
    /// atomically.
    UnknownMailbox {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered mailbox.
        mailbox: MailboxId,
    },
    /// A `SignalUpdate` referenced a `SignalId` that is not registered
    /// in the runtime's signal registry. Aborts the entire batch
    /// atomically.
    UnknownSignal {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered signal.
        signal: SignalId,
    },
    /// A `WakeUnit` referenced a `UnitId` that is not registered in
    /// the runtime's unit registry. Aborts the entire batch
    /// atomically.
    UnknownWakeTarget {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered unit.
        target: UnitId,
    },
    /// A source-side effect (`MailboxReceiveAttempt`, `WaitOnEvent`,
    /// `ReservationAcquire`, or `ConditionalStore`) referenced a
    /// `UnitId` that is not registered. Rejecting these up front
    /// prevents ghost entries from polluting the reservation table
    /// and stops message/status deliveries from silently no-oping
    /// against an invalid id. Aborts the entire batch atomically.
    UnknownSourceUnit {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered unit.
        source: UnitId,
    },
    /// A `DmaEnqueue` targeted a destination range that is not
    /// entirely contained within a single registered memory region
    /// (or whose end address overflows `u64`). Rejecting at enqueue
    /// time surfaces the bad-target at the offending unit's step
    /// rather than later when the completion fires and
    /// `apply_commit` rejects the destination far from the
    /// originating context.
    ///
    /// Only the destination is validated. Source ranges may
    /// legitimately reference SPU local stores, staging buffers,
    /// or other non-guest-memory regions that the completion
    /// handler resolves by path (payload override, MFC channel,
    /// etc.); pre-validating source against the guest-memory
    /// region map would reject those legitimate flows.
    DmaDestinationOutOfRange {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// The underlying memory layer rejected the drain. Reachable when
    /// a staged write lands in a region whose permissions reject
    /// writes, or when pre-validation and drain disagree about
    /// containment. Surfaced rather than panicked so tests and tooling
    /// can assert on it.
    Memory(MemError),
}

/// Summary of what a commit pass accomplished.
///
/// Returned by [`CommitPipeline::process`] on success. Counts the
/// writes that became visible, the effects that were deferred (passed
/// through unhandled), and whether the entire batch was discarded
/// because the originating step yielded with a fault.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitOutcome {
    /// Number of `SharedWriteIntent` effects that were validated,
    /// staged, and applied to committed memory.
    pub writes_committed: usize,
    /// Number of `MailboxSend` effects that were validated and pushed
    /// onto their target mailbox FIFOs.
    pub mailbox_sends_committed: usize,
    /// Number of `SignalUpdate` effects that were validated and
    /// OR-merged into their target signal-notification registers.
    pub signal_updates_committed: usize,
    /// Number of `MailboxReceiveAttempt` effects where the mailbox was
    /// non-empty and a message was popped and delivered to the unit's
    /// pending-receives inbox.
    pub mailbox_receives_committed: usize,
    /// Number of `MailboxReceiveAttempt` effects where the mailbox was
    /// empty, causing the source unit to be blocked.
    pub mailbox_receives_blocked: usize,
    /// Number of `DmaEnqueue` effects that were scheduled into the
    /// DMA completion queue via the latency model.
    pub dma_enqueued: usize,
    /// Number of `WakeUnit` effects that transitioned their target
    /// unit to `Runnable` via a status override.
    pub wakes_committed: usize,
    /// Number of `WaitOnEvent` effects that transitioned their source
    /// unit to `Blocked` via a status override.
    pub waits_committed: usize,
    /// Number of previously-enqueued DMA completions that fired at
    /// this commit boundary (`completion_time <= now`). The runtime
    /// applies their transfers and wakes their issuers.
    pub dma_completions_fired: usize,
    /// Number of `ReservationAcquire` effects that installed or
    /// replaced a reservation entry. Counts attempts that
    /// succeeded at the table level, not net-new entries: a
    /// second acquire on the same unit (whether the raw
    /// `line_addr` canonicalizes to the same line or a different
    /// one) bumps this counter, bumps `reservations_cleared` on
    /// the replacement, and leaves the table size unchanged.
    /// Tooling that cares about net installs subtracts
    /// `reservations_cleared`'s acquire-clobber contribution; the
    /// commit pipeline does not split that sub-count out.
    pub reservation_acquires_committed: usize,
    /// Number of `ConditionalStore` effects whose write was applied
    /// to committed memory. These are always successful stores by
    /// construction -- the emitting unit decides success before
    /// emission.
    pub conditional_stores_committed: usize,
    /// Number of `ConditionalStore` effects that reached the apply
    /// path without a prior reservation entry for the emitter.
    /// Under LL/SC semantics a real emitter must hold a
    /// reservation at store time; reaching the apply path with no
    /// entry means either (a) the emitter is a synthetic test
    /// unit (e.g. `FakeIsaUnit`) that does not model the local
    /// pre-check, or (b) a real emitter has an ordering bug. The
    /// pipeline commits the store either way (soft contract --
    /// see `process` doc) but surfaces the count so real-emitter
    /// CI can assert this counter stays 0 across whole scenarios.
    pub conditional_stores_without_prior_reservation: usize,
    /// Number of reservation-table entries dropped during this
    /// commit. Three sources contribute:
    ///
    /// 1. `SharedWriteIntent` clear-sweep drops every entry whose
    ///    line overlaps the written range.
    /// 2. `ConditionalStore` drops the emitter's own entry plus
    ///    every other unit's entry whose line overlaps the store.
    /// 3. `ReservationAcquire` clobber: when a unit acquires a
    ///    second reservation before releasing the first, the
    ///    prior entry is silently overwritten; that drop counts
    ///    here so replay tooling can see an entry disappeared
    ///    from the table.
    ///
    /// Invariant: `reservations_cleared` equals the difference
    /// between reservation entries that existed at start-of-
    /// commit and entries that exist at end-of-commit, minus net
    /// `reservation_acquires_committed` that installed fresh
    /// entries without clobber.
    pub reservations_cleared: usize,
    /// Number of effects of other variants (fault, trace) that the
    /// pipeline saw and passed through unhandled.
    pub effects_deferred: usize,
    /// `true` if the originating step yielded with `YieldReason::Fault`
    /// and the entire batch was discarded because the step faulted.
    pub fault_discarded: bool,
    /// Number of effects dropped by the fault rule. Zero on any
    /// successful commit; equals `effects.len()` when
    /// `fault_discarded` is `true`. Kept as a scalar rather than a
    /// full per-kind breakdown because per-effect kinds are already
    /// emitted into the trace stream via
    /// `TraceRecord::EffectEmitted` at step time; replay tooling
    /// that needs per-kind discard detail joins the trace records
    /// with this outcome.
    pub effects_discarded_on_fault: usize,
    /// Units whose status was overridden to `Blocked` during this
    /// commit, with the reason for the block.
    pub blocked_units: Vec<(UnitId, BlockReason)>,
    /// Units whose status was overridden to `Runnable` during this
    /// commit (wake effect). Does not include DMA completion wakes,
    /// which are tracked by the runtime.
    pub woken_units: Vec<UnitId>,
}

/// Why the commit pipeline blocked a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// `MailboxReceiveAttempt` on an empty mailbox.
    MailboxEmpty,
    /// `WaitOnEvent` effect.
    WaitOnEvent,
}

/// Mutable references to every runtime subsystem the commit pipeline
/// touches. Bundled into a struct so `CommitPipeline::process` takes
/// two arguments instead of eight, and so adding a new subsystem
/// extends this struct rather than widening a function signature.
pub struct CommitContext<'a> {
    /// Committed guest memory.
    pub memory: &'a mut GuestMemory,
    /// Unit registry (for status overrides and receive delivery).
    pub units: &'a mut UnitRegistry,
    /// Mailbox registry (for send/receive).
    pub mailboxes: &'a mut MailboxRegistry,
    /// Signal-notification register registry (for OR-merge updates).
    pub signals: &'a mut SignalRegistry,
    /// DMA completion queue.
    pub dma_queue: &'a mut DmaQueue,
    /// Latency model for computing DMA completion times.
    pub dma_latency: &'a dyn DmaLatencyModel,
    /// Current guest time (used for DMA scheduling).
    pub now: GuestTicks,
    /// Atomic reservation table. Updated in place by
    /// `ReservationAcquire` (insert/replace), `ConditionalStore`
    /// (drop emitter's entry), and by the clear sweep that runs
    /// after every committed write (drops overlapping entries from
    /// other units).
    pub reservations: &'a mut ReservationTable,
    /// Base address of the RSX label area -- the address returned
    /// by `cellGcmGetLabelAddress(0)` and published by
    /// `cellGcmInitBody` into `hle::cell_gcm_sys::GcmState`.
    /// Zero if GCM has not been initialised. An `Effect::RsxLabelWrite`
    /// commits as a 4-byte big-endian write at
    /// `rsx_label_base + offset`; the offset is interpreted against
    /// this base, NOT against the raw guest address space.
    pub rsx_label_base: u32,
    /// RSX flip-status state machine. Mutated by `RsxFlipRequest`
    /// effect processing (sets WAITING + pending + records buffer
    /// index) and by the per-boundary DONE transition fired from
    /// `Runtime::commit_step` after a pending flip survives one
    /// full batch. Reads are not required by the commit pipeline;
    /// the pipeline only writes here.
    pub rsx_flip: &'a mut crate::rsx_flip::RsxFlipState,
}

/// The commit pipeline.
///
/// Holds no persistent state (the staging buffer is per-call because
/// there is one commit batch per unit yield with no cross-unit
/// batching). The struct exists so future state can be added without
/// changing the public API shape.
#[derive(Debug, Default)]
pub struct CommitPipeline {}

impl CommitPipeline {
    /// Construct a fresh commit pipeline.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process the effects produced by a single unit step.
    ///
    /// Behavior:
    ///
    /// 1. If `result.yield_reason == YieldReason::Fault`, every effect
    ///    is discarded (the fault rule). Returns
    ///    `CommitOutcome { fault_discarded: true, .. }`.
    /// 2. Otherwise, walks `result.emitted_effects` in order:
    ///    - `SharedWriteIntent` is validated (length match, in-bounds),
    ///      staged into a fresh [`StagingMemory`]. After the write
    ///      commits the clear sweep drops any reservation entries
    ///      whose line overlaps the write.
    ///    - `MailboxSend` pushes onto the target FIFO.
    ///    - `MailboxReceiveAttempt` pops or blocks the source unit.
    ///    - `SignalUpdate` OR-merges into the target register.
    ///    - `DmaEnqueue` schedules a completion via the latency model.
    ///    - `WakeUnit` overrides the target's status to Runnable.
    ///    - `WaitOnEvent` overrides the source's status to Blocked.
    ///    - `ReservationAcquire` inserts / replaces the source's
    ///      atomic-reservation entry, canonicalized to a 128-byte
    ///      line.
    ///    - `ConditionalStore` applies the write (same validation
    ///      and staging as `SharedWriteIntent`), drops the emitter's
    ///      reservation entry, and runs the clear sweep against
    ///      every other unit's entry covering the line.
    ///    - `FaultRaised` and `TraceMarker` fall through and are
    ///      counted as deferred.
    /// 3. Validation failure aborts the whole batch atomically:
    ///    nothing is committed, the staging buffer (which never reached
    ///    `drain_into`) is discarded with the function return, and the
    ///    error names the offending effect's index.
    /// 4. After all effects are validated and staged, drains the
    ///    staging buffer into `memory`. Either every staged write
    ///    becomes visible or none do (atomic-batch guarantee).
    ///
    /// The `dma_completions_fired` field of the returned outcome is
    /// always zero from this method; `Runtime::commit_step` fills it
    /// in after popping the DMA queue against current guest time.
    pub fn process(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
        ctx: &mut CommitContext<'_>,
    ) -> Result<CommitOutcome, CommitError> {
        if result.yield_reason == YieldReason::Fault {
            return Ok(CommitOutcome {
                fault_discarded: true,
                effects_discarded_on_fault: effects.len(),
                ..CommitOutcome::default()
            });
        }

        // Fast path: no effects means nothing to validate, stage, or apply.
        if effects.is_empty() {
            return Ok(CommitOutcome::default());
        }

        let mut staging = StagingMemory::new();
        let mut writes = 0usize;
        let mut sends = 0usize;
        let mut receives = 0usize;
        let mut receives_blocked = 0usize;
        let mut signal_updates = 0usize;
        let mut dma_count = 0usize;
        let mut wakes = 0usize;
        let mut waits = 0usize;
        let mut reservation_acquires = 0usize;
        let mut conditional_stores = 0usize;
        let mut conditional_stores_without_prior_reservation = 0usize;
        let mut reservations_cleared = 0usize;
        let mut blocked_units = Vec::new();
        let mut woken_units = Vec::new();
        let mut deferred = 0usize;

        // Pre-validation pass. Walk effects in emission order; reject
        // the entire batch on the first failure (atomic-batch rule).
        // Mailbox sends are validated against the registry but not yet
        // applied -- mailbox state mutates only in the apply pass
        // below, which runs after every effect has been validated.
        for (idx, effect) in effects.iter().enumerate() {
            match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    if bytes.len() as u64 != range.length() {
                        return Err(CommitError::PayloadLengthMismatch { effect_index: idx });
                    }
                    let start = range.start().raw();
                    let length = range.length();
                    let _end = start
                        .checked_add(length)
                        .ok_or(CommitError::OutOfRange { effect_index: idx })?;
                    if ctx.memory.containing_region(start, length).is_none() {
                        return Err(CommitError::OutOfRange { effect_index: idx });
                    }
                    staging.stage(StagedWrite {
                        range: *range,
                        bytes: bytes.bytes().to_vec(),
                    });
                    writes += 1;
                }
                Effect::MailboxSend { mailbox, .. } => {
                    if ctx.mailboxes.get(*mailbox).is_none() {
                        return Err(CommitError::UnknownMailbox {
                            effect_index: idx,
                            mailbox: *mailbox,
                        });
                    }
                    sends += 1;
                }
                Effect::MailboxReceiveAttempt {
                    mailbox, source, ..
                } => {
                    if ctx.mailboxes.get(*mailbox).is_none() {
                        return Err(CommitError::UnknownMailbox {
                            effect_index: idx,
                            mailbox: *mailbox,
                        });
                    }
                    // Validate the source: the apply pass calls
                    // push_receive / set_status_override on this id,
                    // which would silently drop the message or
                    // panic mid-commit if the unit is unregistered.
                    if ctx.units.get(*source).is_none() {
                        return Err(CommitError::UnknownSourceUnit {
                            effect_index: idx,
                            source: *source,
                        });
                    }
                }
                Effect::SignalUpdate { signal, .. } => {
                    if ctx.signals.get(*signal).is_none() {
                        return Err(CommitError::UnknownSignal {
                            effect_index: idx,
                            signal: *signal,
                        });
                    }
                    signal_updates += 1;
                }
                Effect::DmaEnqueue { request, .. } => {
                    // Pre-validate the destination range: the
                    // completion path will `apply_commit` to this
                    // range far from the originating step, and a
                    // bad target there would surface as an obscure
                    // commit failure with no link back to the
                    // emitter. Source may legitimately be a local
                    // store or staging buffer outside the guest
                    // memory regions, so we only check the
                    // destination.
                    let dst = request.destination();
                    let start = dst.start().raw();
                    let length = dst.length();
                    if start.checked_add(length).is_none()
                        || ctx.memory.containing_region(start, length).is_none()
                    {
                        return Err(CommitError::DmaDestinationOutOfRange { effect_index: idx });
                    }
                    dma_count += 1;
                }
                Effect::WakeUnit { target, .. } => {
                    if ctx.units.get(*target).is_none() {
                        return Err(CommitError::UnknownWakeTarget {
                            effect_index: idx,
                            target: *target,
                        });
                    }
                    wakes += 1;
                }
                Effect::WaitOnEvent { source, .. } => {
                    // Validate source: apply pass calls
                    // set_status_override on this id. An
                    // unregistered source either silently no-ops
                    // or panics mid-commit (see the similar
                    // concern on MailboxReceiveAttempt).
                    if ctx.units.get(*source).is_none() {
                        return Err(CommitError::UnknownSourceUnit {
                            effect_index: idx,
                            source: *source,
                        });
                    }
                    waits += 1;
                }
                Effect::ConditionalStore {
                    range,
                    bytes,
                    source,
                    ..
                } => {
                    // Same validation rules as SharedWriteIntent:
                    // payload length matches range, range lies fully
                    // within one registered region. A failure here
                    // aborts the whole batch atomically.
                    if bytes.len() as u64 != range.length() {
                        return Err(CommitError::PayloadLengthMismatch { effect_index: idx });
                    }
                    let start = range.start().raw();
                    let length = range.length();
                    let _end = start
                        .checked_add(length)
                        .ok_or(CommitError::OutOfRange { effect_index: idx })?;
                    if ctx.memory.containing_region(start, length).is_none() {
                        return Err(CommitError::OutOfRange { effect_index: idx });
                    }
                    // Validate source: the apply pass calls
                    // remove_if_present on this id to drop the
                    // emitter's reservation entry. An unregistered
                    // source would silently bypass the
                    // "drop emitter's entry" invariant and the
                    // store would commit anyway -- worse than
                    // rejecting.
                    if ctx.units.get(*source).is_none() {
                        return Err(CommitError::UnknownSourceUnit {
                            effect_index: idx,
                            source: *source,
                        });
                    }
                    staging.stage(StagedWrite {
                        range: *range,
                        bytes: bytes.bytes().to_vec(),
                    });
                    conditional_stores += 1;
                }
                Effect::ReservationAcquire { source, .. } => {
                    // Validate source: an unregistered id would
                    // still populate the reservation table (the
                    // table has no registry awareness), producing
                    // a ghost entry that survives until some other
                    // unit's overlapping write sweeps it. Rejecting
                    // here keeps the table registry-consistent
                    // with the rest of the pipeline.
                    if ctx.units.get(*source).is_none() {
                        return Err(CommitError::UnknownSourceUnit {
                            effect_index: idx,
                            source: *source,
                        });
                    }
                }
                Effect::RsxLabelWrite { offset, value } => {
                    // Resolve against the RSX label base and stage
                    // as a 4-byte big-endian write. When label_base
                    // is zero, the offset is treated as an
                    // absolute guest address -- the
                    // `containing_region` check below still rejects
                    // writes outside mapped memory, so a stray
                    // offset produces a commit error rather than
                    // corrupting random addresses. Microtests that
                    // skip cellGcmInit and allocate their own
                    // label region use this path; foundation
                    // titles that call cellGcmInit get a non-zero
                    // label_base and the offset is interpreted
                    // relative to it.
                    let start = (ctx.rsx_label_base as u64).wrapping_add(*offset as u64);
                    let Some(_end) = start.checked_add(4) else {
                        return Err(CommitError::OutOfRange { effect_index: idx });
                    };
                    if ctx.memory.containing_region(start, 4).is_none() {
                        return Err(CommitError::OutOfRange { effect_index: idx });
                    }
                    let Ok(range) =
                        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(start), 4)
                            .ok_or(CommitError::OutOfRange { effect_index: idx })
                    else {
                        return Err(CommitError::OutOfRange { effect_index: idx });
                    };
                    staging.stage(StagedWrite {
                        range,
                        bytes: value.to_be_bytes().to_vec(),
                    });
                    writes += 1;
                }
                _ => {
                    deferred += 1;
                }
            }
        }

        // Apply pass. Memory writes drain atomically through the
        // staging buffer, then mailbox sends push onto their FIFOs,
        // signal updates OR-merge into their registers, and DMA
        // requests are scheduled into the completion queue via the
        // latency model. All happen in the same emission order the
        // units produced them. Pre-validation guarantees every
        // lookup here succeeds.
        //
        // Atomicity contract. The `drain_into` below is the only
        // operation between here and function return that can
        // represent failure as `CommitError`. The ops that run
        // after it (mb.send, signal.or_in, dma_queue.enqueue,
        // reservations.*, payload.clone in DmaEnqueue) are
        // infallible in the current design:
        //
        //   - `Mailbox::send` pushes onto a Vec.
        //   - `Signal::or_in` is a bitwise OR into a register.
        //   - `DmaQueue::enqueue` pushes onto a BinaryHeap.
        //   - `DmaLatencyModel::completion_time` is a pure function.
        //   - `ReservationTable` ops are BTreeMap operations.
        //   - `payload.clone` copies a `Vec<u8>`.
        //
        // None of these can fail absent host-side allocation
        // failure. The project's stance is: host OOM aborts the
        // process via the Rust allocator's default behavior; we
        // do not model it as a recoverable error and no
        // `CommitError` variant represents it. The module-level
        // atomicity guarantee ("either every staged write becomes
        // visible or none do") is predicated on that assumption;
        // under host OOM the process is already dying and guest-
        // visible consistency is no longer meaningful.
        //
        // The `.expect(...)` calls below likewise cannot fire
        // because `ctx` is held by `&mut` for the duration of
        // this call and nothing external can deregister a
        // mailbox/signal between the pre-validation and apply
        // passes.
        //
        // If any of those guarantees change (new registry impl,
        // new latency model with fallible ops, a future
        // representation of host-OOM as a recoverable error), the
        // atomicity contract from the module-level doc becomes
        // load-bearing on behavior that post-dates `drain_into`,
        // and rollback machinery becomes necessary. Audit this
        // block when adding a new fallible op in the apply pass.
        staging
            .drain_into(ctx.memory)
            .map_err(CommitError::Memory)?;
        for effect in effects {
            match effect {
                Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    ctx.mailboxes
                        .get_mut(*mailbox)
                        .expect("pre-validated mailbox id")
                        .send(message.raw());
                }
                Effect::SignalUpdate { signal, value, .. } => {
                    ctx.signals
                        .get_mut(*signal)
                        .expect("pre-validated signal id")
                        .or_in(*value);
                }
                Effect::DmaEnqueue { request, payload } => {
                    let completion_time = ctx.dma_latency.completion_time(request, ctx.now);
                    let completion = DmaCompletion::new(*request, completion_time);
                    ctx.dma_queue.enqueue(completion, payload.clone());
                }
                Effect::MailboxReceiveAttempt {
                    mailbox, source, ..
                } => {
                    let mb = ctx
                        .mailboxes
                        .get_mut(*mailbox)
                        .expect("pre-validated mailbox id");
                    match mb.try_receive() {
                        Some(msg) => {
                            ctx.units.push_receive(*source, msg);
                            receives += 1;
                        }
                        None => {
                            ctx.units.set_status_override(*source, UnitStatus::Blocked);
                            blocked_units.push((*source, BlockReason::MailboxEmpty));
                            receives_blocked += 1;
                        }
                    }
                }
                Effect::WakeUnit { target, .. } => {
                    ctx.units.set_status_override(*target, UnitStatus::Runnable);
                    woken_units.push(*target);
                }
                Effect::WaitOnEvent { source, .. } => {
                    ctx.units.set_status_override(*source, UnitStatus::Blocked);
                    blocked_units.push((*source, BlockReason::WaitOnEvent));
                }
                Effect::SharedWriteIntent { range, .. } => {
                    // Clear sweep: any committed write to a reserved
                    // line drops every unit's entry covering the
                    // line. Runs in the apply pass so cross-unit
                    // invalidation follows the same emission order
                    // as the write itself.
                    reservations_cleared += ctx
                        .reservations
                        .clear_covering(range.start().raw(), range.length());
                }
                Effect::ReservationAcquire { line_addr, source } => {
                    // Canonicalize to line granule at insert time.
                    // The unit may have passed the raw EA; the table
                    // only ever stores line-aligned addresses.
                    //
                    // A prior reservation for the same unit is
                    // clobbered. That is legal under LL/SC
                    // semantics (a new lwarx/getllar overwrites the
                    // previous reservation) but the clobber still
                    // counts as a reservation dropped at this
                    // commit boundary, so replay and audit tooling
                    // can tell that an entry disappeared. Bumping
                    // `reservations_cleared` on the prior-entry
                    // return keeps the counter's invariant
                    // "reservations that were in the table at
                    // start-of-commit and are no longer at
                    // end-of-commit."
                    let prior = ctx
                        .reservations
                        .insert_or_replace(*source, ReservedLine::containing(*line_addr));
                    if prior.is_some() {
                        reservations_cleared += 1;
                    }
                    reservation_acquires += 1;
                }
                Effect::ConditionalStore { range, source, .. } => {
                    // The write has already been drained into memory
                    // by the staging pass above. Drop the emitter's
                    // own reservation entry, then run the clear sweep
                    // against every OTHER unit's entries covering
                    // the line. Order matters: removing the emitter
                    // first means its entry is never subject to the
                    // sweep (avoids double-counting in
                    // `reservations_cleared`).
                    //
                    // Contract note: the pipeline does not verify
                    // that the emitter's reservation was still held
                    // (or that it covered the store line) at apply
                    // time. `ConditionalStore` is a soft emitter-side
                    // contract -- the emitting unit decides success
                    // before emission based on its own local
                    // reservation register. A cross-unit
                    // SharedWriteIntent earlier in the same batch
                    // can in principle clear the emitter's table
                    // entry between emission and this apply point.
                    // Synthetic emitters (e.g. `FakeIsaUnit` in
                    // tests) may not model the local pre-check and
                    // will reach here with no prior entry; that is
                    // not an invariant violation at the pipeline
                    // layer. Real emitters (`cellgov_ppu`,
                    // `cellgov_spu`) pre-check; if they ever reach
                    // here without a prior entry, that is an
                    // emitter-side bug to fix there, not here.
                    if ctx.reservations.remove_if_present(*source).is_some() {
                        reservations_cleared += 1;
                    } else {
                        // Observability hook for emitter-side LL/SC
                        // bugs. Synthetic emitters (FakeIsaUnit) hit
                        // this every time; real emitter scenarios
                        // must assert this counter stays 0.
                        conditional_stores_without_prior_reservation += 1;
                    }
                    reservations_cleared += ctx
                        .reservations
                        .clear_covering(range.start().raw(), range.length());
                }
                Effect::RsxFlipRequest { buffer_index } => {
                    // Transition the flip state: status -> WAITING,
                    // pending -> true, record buffer_index. The
                    // WAITING -> DONE transition fires at the next
                    // commit boundary via Runtime::commit_step's
                    // post-apply hook. If a flip was ALREADY
                    // pending when this effect arrives, the corner
                    // case is last-writer-wins on buffer_index,
                    // pending stays true, status stays WAITING.
                    // `request_flip` implements that policy.
                    ctx.rsx_flip.request_flip(*buffer_index);
                }
                _ => {}
            }
        }

        Ok(CommitOutcome {
            writes_committed: writes,
            mailbox_sends_committed: sends,
            mailbox_receives_committed: receives,
            mailbox_receives_blocked: receives_blocked,
            signal_updates_committed: signal_updates,
            dma_enqueued: dma_count,
            wakes_committed: wakes,
            waits_committed: waits,
            dma_completions_fired: 0, // set by Runtime::commit_step
            reservation_acquires_committed: reservation_acquires,
            conditional_stores_committed: conditional_stores,
            conditional_stores_without_prior_reservation,
            reservations_cleared,
            effects_deferred: deferred,
            fault_discarded: false,
            effects_discarded_on_fault: 0,
            blocked_units,
            woken_units,
        })
    }
}

#[cfg(test)]
#[path = "tests/commit_tests.rs"]
mod tests;
