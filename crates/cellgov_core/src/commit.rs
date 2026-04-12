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
//! (status override to Runnable), and `WaitOnEvent` (status override
//! to Blocked). `FaultRaised` and `TraceMarker` are counted but do
//! not mutate runtime state.

use crate::registry::UnitRegistry;
use cellgov_dma::{DmaCompletion, DmaLatencyModel, DmaQueue};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus, YieldReason};
use cellgov_mem::{GuestMemory, MemError, StagedWrite, StagingMemory};
use cellgov_sync::{MailboxId, MailboxRegistry, SignalId, SignalRegistry};
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
    /// A `SharedWriteIntent`'s target range extends past the end of
    /// committed memory or its end address overflows `u64`.
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
    /// The underlying memory layer rejected the drain. Should be
    /// unreachable in practice given the pre-validation pass, but
    /// surfaced rather than panicked so tests and tooling can assert
    /// on it.
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
    /// applies their transfers and wakes their issuers. Set by
    /// `Runtime::commit_step`, not by `CommitPipeline::process`.
    pub dma_completions_fired: usize,
    /// Number of effects of other variants (fault, trace) that the
    /// pipeline saw and passed through unhandled.
    pub effects_deferred: usize,
    /// `true` if the originating step yielded with `YieldReason::Fault`
    /// and the entire batch was discarded because the step faulted.
    pub fault_discarded: bool,
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
}

/// The commit pipeline.
///
/// Holds no persistent state (the staging buffer is per-call because
/// there is one commit batch per unit yield with no cross-unit
/// batching). The struct exists so future state can be
/// added (event queue handle, sync state references) without changing
/// the public API shape.
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
    ///      staged into a fresh [`StagingMemory`].
    ///    - Other variants are counted as deferred.
    /// 3. Validation failure aborts the whole batch atomically:
    ///    nothing is committed, the staging buffer (which never reached
    ///    `drain_into`) is discarded with the function return, and the
    ///    error names the offending effect's index.
    /// 4. After all effects are validated and staged, drains the
    ///    staging buffer into `memory`. Either every staged write
    ///    becomes visible or none do (atomic-batch guarantee).
    pub fn process(
        &mut self,
        result: &ExecutionStepResult,
        ctx: &mut CommitContext<'_>,
    ) -> Result<CommitOutcome, CommitError> {
        if result.yield_reason == YieldReason::Fault {
            return Ok(CommitOutcome {
                fault_discarded: true,
                ..CommitOutcome::default()
            });
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
        let mut blocked_units = Vec::new();
        let mut woken_units = Vec::new();
        let mut deferred = 0usize;
        let mem_size = ctx.memory.size();

        // Pre-validation pass. Walk effects in emission order; reject
        // the entire batch on the first failure (atomic-batch rule).
        // Mailbox sends are validated against the registry but not yet
        // applied -- mailbox state mutates only in the apply pass
        // below, which runs after every effect has been validated.
        for (idx, effect) in result.emitted_effects.iter().enumerate() {
            match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    if bytes.len() as u64 != range.length() {
                        return Err(CommitError::PayloadLengthMismatch { effect_index: idx });
                    }
                    let end = range
                        .start()
                        .raw()
                        .checked_add(range.length())
                        .ok_or(CommitError::OutOfRange { effect_index: idx })?;
                    if end > mem_size {
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
                Effect::MailboxReceiveAttempt { mailbox, .. } => {
                    if ctx.mailboxes.get(*mailbox).is_none() {
                        return Err(CommitError::UnknownMailbox {
                            effect_index: idx,
                            mailbox: *mailbox,
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
                Effect::DmaEnqueue { .. } => {
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
                Effect::WaitOnEvent { .. } => {
                    waits += 1;
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
        // units produced them. Pre-validation guarantees every lookup
        // here succeeds.
        staging
            .drain_into(ctx.memory)
            .map_err(CommitError::Memory)?;
        for effect in &result.emitted_effects {
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
            effects_deferred: deferred,
            fault_discarded: false,
            blocked_units,
            woken_units,
        })
    }
}

#[cfg(test)]
#[path = "tests/commit_tests.rs"]
mod tests;
