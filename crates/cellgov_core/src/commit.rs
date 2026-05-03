//! Commit pipeline: validate, stage, apply a unit's emitted effects.
//!
//! Contract (none of these are expressible in the type system):
//!
//! - One commit batch per unit yield; no cross-unit batching.
//! - Atomic from guest visibility: every `SharedWriteIntent` in the
//!   batch becomes visible at the same epoch boundary, or none do.
//! - Fault rule: `YieldReason::Fault` discards the whole batch,
//!   including effects emitted before the fault.
//! - Validation rejects the whole batch; a rejected batch commits
//!   nothing and surfaces as a fault on the originating unit.
//! - Every committed `SharedWriteIntent` runs the reservation-table
//!   clear sweep against overlapping lines.
//! - `RsxLabelWrite` is bounds-checked against the resolved label
//!   base before staging.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitError {
    /// A `SharedWriteIntent` payload length did not match its range length.
    PayloadLengthMismatch {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// A `SharedWriteIntent` target range escapes any registered region.
    OutOfRange {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// A `MailboxSend` targeted an unregistered mailbox.
    UnknownMailbox {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered mailbox.
        mailbox: MailboxId,
    },
    /// A `SignalUpdate` targeted an unregistered signal.
    UnknownSignal {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered signal.
        signal: SignalId,
    },
    /// A `WakeUnit` targeted an unregistered unit.
    UnknownWakeTarget {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered unit.
        target: UnitId,
    },
    /// A source-side effect named an unregistered unit; rejecting keeps
    /// the reservation table and pending-receive inbox registry-consistent.
    UnknownSourceUnit {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
        /// The unregistered unit.
        source: UnitId,
    },
    /// A `DmaEnqueue` destination range escapes any registered region.
    ///
    /// Only destination is pre-validated; source ranges may legitimately
    /// reference SPU local stores or staging buffers that the completion
    /// handler resolves by path.
    DmaDestinationOutOfRange {
        /// Position of the offending effect in `emitted_effects`.
        effect_index: usize,
    },
    /// The memory layer rejected the drain (permissions, or a
    /// pre-validation/drain disagreement on containment).
    Memory(MemError),
}

/// Summary of what a commit pass accomplished.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitOutcome {
    /// Committed `SharedWriteIntent` count (includes `RsxLabelWrite`).
    pub writes_committed: usize,
    /// Committed `MailboxSend` count.
    pub mailbox_sends_committed: usize,
    /// Committed `SignalUpdate` count.
    pub signal_updates_committed: usize,
    /// `MailboxReceiveAttempt`s that popped a message.
    pub mailbox_receives_committed: usize,
    /// `MailboxReceiveAttempt`s that blocked on an empty mailbox.
    pub mailbox_receives_blocked: usize,
    /// `DmaEnqueue`s scheduled into the completion queue.
    pub dma_enqueued: usize,
    /// `WakeUnit`s applied as `Runnable` overrides.
    pub wakes_committed: usize,
    /// `WaitOnEvent`s applied as `Blocked` overrides.
    pub waits_committed: usize,
    /// DMA completions that fired at this boundary. Filled by the runtime,
    /// always zero from [`CommitPipeline::process`].
    pub dma_completions_fired: usize,
    /// `ReservationAcquire`s that installed or replaced an entry.
    ///
    /// A second acquire on the same unit bumps this counter AND
    /// `reservations_cleared`; the table size is unchanged. Tooling that
    /// wants net installs must reconcile both counters.
    pub reservation_acquires_committed: usize,
    /// `ConditionalStore`s whose write was applied.
    pub conditional_stores_committed: usize,
    /// `ConditionalStore`s that reached apply without a prior reservation
    /// for the emitter. Zero for correct real emitters (PPU, SPU); non-zero
    /// for synthetic test units that skip the LL/SC pre-check, or an
    /// emitter-side ordering bug. Whole-scenario CI asserts zero for real
    /// emitters.
    pub conditional_stores_without_prior_reservation: usize,
    /// Reservation-table entries dropped during this commit.
    ///
    /// Sources: `SharedWriteIntent` clear-sweep, `ConditionalStore`
    /// emitter-entry drop and cross-unit sweep, and `ReservationAcquire`
    /// clobbers of prior entries on the same unit.
    pub reservations_cleared: usize,
    /// Effects the pipeline saw and did not act on (fault, trace).
    pub effects_deferred: usize,
    /// `true` if the step faulted and the whole batch was discarded.
    pub fault_discarded: bool,
    /// Count of effects dropped by the fault rule. Equals `effects.len()`
    /// when `fault_discarded`, else zero. Per-kind detail lives in the
    /// trace stream.
    pub effects_discarded_on_fault: usize,
    /// Units blocked during this commit.
    pub blocked_units: Vec<(UnitId, BlockReason)>,
    /// Units woken during this commit (excludes DMA-completion wakes).
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

/// Mutable references to every subsystem the commit pipeline touches.
pub struct CommitContext<'a> {
    /// Committed guest memory.
    pub memory: &'a mut GuestMemory,
    /// Unit registry (status overrides, receive delivery).
    pub units: &'a mut UnitRegistry,
    /// Mailbox registry.
    pub mailboxes: &'a mut MailboxRegistry,
    /// Signal-notification register registry.
    pub signals: &'a mut SignalRegistry,
    /// DMA completion queue.
    pub dma_queue: &'a mut DmaQueue,
    /// Latency model for DMA completion times.
    pub dma_latency: &'a dyn DmaLatencyModel,
    /// Current guest time.
    pub now: GuestTicks,
    /// Atomic reservation table.
    pub reservations: &'a mut ReservationTable,
    /// RSX label area base address; zero means GCM has not been initialised.
    /// `RsxLabelWrite` commits as a 4-byte big-endian store at
    /// `rsx_label_base + offset`.
    pub rsx_label_base: u32,
    /// RSX flip-status state machine; write-only from this pipeline.
    pub rsx_flip: &'a mut crate::rsx::flip::RsxFlipState,
}

/// The commit pipeline.
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
    /// See the module docs for the full contract (atomic-batch,
    /// fault-discards-all, validation-rejects-whole-batch). Validation
    /// runs over every effect first; staged memory writes drain as one
    /// atomic operation before any other subsystem is mutated.
    ///
    /// # Errors
    ///
    /// Returns `CommitError` on validation failure or memory-drain
    /// rejection; the batch commits nothing on error.
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

        // Pre-validation pass: reject the whole batch on the first
        // failure. Subsystem state mutates only in the apply pass below.
        // The IIFE wrapping lets a validation failure return from the
        // closure rather than the outer function so we can `staging.clear()`
        // before propagating -- StagingMemory's Drop debug-asserts that the
        // buffer is empty at release time, and an early-return path that
        // skipped the clear would trip it.
        let pre_validate: Result<(), CommitError> = (|| {
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
                        // Pre-validate only the destination; source ranges
                        // may legitimately reference SPU local stores or
                        // staging buffers outside the region map.
                        let dst = request.destination();
                        let start = dst.start().raw();
                        let length = dst.length();
                        if start.checked_add(length).is_none()
                            || ctx.memory.containing_region(start, length).is_none()
                        {
                            return Err(CommitError::DmaDestinationOutOfRange {
                                effect_index: idx,
                            });
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
                        if ctx.units.get(*source).is_none() {
                            return Err(CommitError::UnknownSourceUnit {
                                effect_index: idx,
                                source: *source,
                            });
                        }
                    }
                    Effect::RsxLabelWrite { offset, value } => {
                        // Offsets 0..0x1000 are the semaphore region; 0x1000+
                        // is notify/report space under sys_rsx and is a guest
                        // bug. Surface at the commit boundary rather than as
                        // silent notify corruption.
                        debug_assert!(
                            *offset < 0x1000,
                            "RsxLabelWrite offset {:#x} past semaphore region (guest bug? \
                         0..0x1000 is semaphore, 0x1000+ is notify/report)",
                            *offset
                        );
                        // rsx_label_base: u32 and offset: u32 widened to u64
                        // before adding, so `start <= 2 * u32::MAX < u64::MAX`
                        // and the wrap is structurally unreachable for any
                        // valid input. `wrapping_add` documents the
                        // arithmetic; `checked_add` on the +4 below remains
                        // because `start + 4` could in principle reach
                        // `u64::MAX` if the inputs were ever extended to u64.
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
            Ok(())
        })();
        if let Err(e) = pre_validate {
            staging.clear();
            return Err(e);
        }

        // Apply pass. `drain_into` is the only operation below that
        // can fail; every op that follows (mailbox send, signal OR,
        // DMA enqueue, reservation mutations, payload clone) is
        // infallible absent host OOM. Adding a new fallible op here
        // breaks the atomicity guarantee and needs rollback machinery.
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
                    // Cross-unit reservation invalidation in emission order.
                    reservations_cleared += ctx
                        .reservations
                        .clear_covering(range.start().raw(), range.length());
                }
                Effect::ReservationAcquire { line_addr, source } => {
                    // Canonicalize to 128-byte line at insert; callers may
                    // pass a raw EA. A prior entry on the same unit is
                    // clobbered (LL/SC re-reservation) and counted as cleared.
                    let prior = ctx
                        .reservations
                        .insert_or_replace(*source, ReservedLine::containing(*line_addr));
                    if prior.is_some() {
                        reservations_cleared += 1;
                    }
                    reservation_acquires += 1;
                }
                Effect::ConditionalStore { range, source, .. } => {
                    // Drop the emitter's own entry first so it is never
                    // double-counted by the subsequent cross-unit sweep.
                    //
                    // Soft contract: the pipeline does not re-check that
                    // the emitter still holds a reservation covering the
                    // line. Synthetic emitters reach this branch with no
                    // prior entry and bump the unconditional counter; real
                    // emitters hitting that path indicate an emitter-side
                    // LL/SC bug.
                    if ctx.reservations.remove_if_present(*source).is_some() {
                        reservations_cleared += 1;
                    } else {
                        conditional_stores_without_prior_reservation += 1;
                    }
                    reservations_cleared += ctx
                        .reservations
                        .clear_covering(range.start().raw(), range.length());
                }
                Effect::RsxFlipRequest { buffer_index } => {
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
