//! Commit pipeline: validate, stage, apply a unit's emitted effects.
//!
//! Cross-module contract:
//!
//! - One commit batch per unit yield; no cross-unit batching.
//! - Atomic guest visibility: every `SharedWriteIntent` in the batch
//!   becomes visible at the same epoch boundary, or none do.
//! - `YieldReason::Fault` discards the whole batch, including effects
//!   emitted before the fault.
//! - Validation rejects the whole batch; a rejected batch commits
//!   nothing and surfaces as a fault on the originating unit.
//! - Every committed `SharedWriteIntent` runs the reservation-table
//!   clear sweep against overlapping lines. So does `RsxLabelWrite`,
//!   with no unit exempted: the RSX is a bus master, not a unit.
//! - `RsxLabelWrite` is bounds-checked against the resolved label
//!   base before staging.

use crate::registry::UnitRegistry;
use cellgov_dma::{DmaCompletion, DmaDirection, DmaLatencyModel, DmaQueue};
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
/// No variant is a unit variant: each carries the effect index it
/// refused, and `Memory` carries the memory layer's own error instead.
/// So `VariantArray` cannot publish the set, and `EnumCount` publishes
/// how many there are -- what a sweep standing one refusal in for every
/// shape needs to know it still covers them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, strum::EnumCount)]
pub enum CommitError {
    /// A `SharedWriteIntent` payload length did not match its range length.
    #[error("effect[{effect_index}]: write payload length disagrees with range")]
    PayloadLengthMismatch {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `SharedWriteIntent` target range escapes any registered region.
    #[error("effect[{effect_index}]: write target escapes regions")]
    OutOfRange {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `MailboxSend` or `MailboxReceiveAttempt` targeted an unregistered mailbox.
    #[error("effect[{effect_index}]: unknown mailbox {mailbox}")]
    UnknownMailbox {
        /// Index of the offending effect within the batch.
        effect_index: usize,
        /// Mailbox id that was not found in the registry.
        mailbox: MailboxId,
    },
    /// A `SignalUpdate` targeted an unregistered signal.
    #[error("effect[{effect_index}]: unknown signal {signal}")]
    UnknownSignal {
        /// Index of the offending effect within the batch.
        effect_index: usize,
        /// Signal id that was not found in the registry.
        signal: SignalId,
    },
    /// A `WakeUnit` targeted an unregistered unit.
    #[error("effect[{effect_index}]: unknown wake target unit {}", target.raw())]
    UnknownWakeTarget {
        /// Index of the offending effect within the batch.
        effect_index: usize,
        /// Target unit id that was not found in the registry.
        target: UnitId,
    },
    /// A source-side effect named an unregistered unit; rejecting keeps
    /// the reservation table and pending-receive inbox registry-consistent.
    #[error("effect[{effect_index}]: unknown source unit {}", source_unit.raw())]
    UnknownSourceUnit {
        /// Index of the offending effect within the batch.
        effect_index: usize,
        /// Source unit id that was not found in the registry.
        source_unit: UnitId,
    },
    /// A `DmaEnqueue` destination range escapes any registered region.
    #[error("effect[{effect_index}]: DMA destination escapes regions")]
    DmaDestinationOutOfRange {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `DmaEnqueue` carrying no inline payload names a source range
    /// that escapes any registered region.
    ///
    /// The completion reads the source out of committed space 0, so a
    /// range that does not resolve there has no bytes to move. An
    /// enqueue that carries its bytes inline is not held to this: the
    /// completion never reads a range for it.
    #[error("effect[{effect_index}]: DMA source escapes regions")]
    DmaSourceOutOfRange {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `DmaEnqueue` inline payload is not as long as the destination
    /// it lands in.
    ///
    /// The completion writes the payload over the whole destination
    /// range, so the two lengths are one fact. A disagreement reaches
    /// the memory layer as a length mismatch the completion has no
    /// refusal for.
    #[error("effect[{effect_index}]: DMA payload length disagrees with the destination")]
    DmaPayloadLengthMismatch {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `DmaEnqueue` names a direction the completion does not model.
    ///
    /// The completion writes the request's destination into committed
    /// space 0 whatever the direction says, which is the `Put` reading.
    /// A `Get` names the local-store end as its destination, so
    /// applying it there would land the transfer in main memory at a
    /// local-store address. The queue refuses it rather than modelling
    /// it wrongly.
    #[error("effect[{effect_index}]: DMA direction is not modelled")]
    DmaDirectionUnsupported {
        /// Index of the offending effect within the batch.
        effect_index: usize,
    },
    /// A `DmaEnqueue` destination range lies in a non-`ReadWrite` region.
    #[error(
        "effect[{effect_index}]: DMA destination at 0x{addr:016x} lies in \
         reserved region {region}"
    )]
    DmaDestinationReserved {
        /// Index of the offending effect within the batch.
        effect_index: usize,
        /// Faulting guest address (start of destination range).
        addr: u64,
        /// Reserved region's label.
        region: &'static str,
    },
    /// The memory layer rejected the drain (permissions, or a
    /// pre-validation/drain disagreement on containment).
    #[error("memory: {0}")]
    Memory(#[source] MemError),
}

/// Summary of what a commit pass accomplished.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitOutcome {
    /// Includes `RsxLabelWrite` (staged as a 4-byte BE store).
    pub writes_committed: usize,
    /// Number of `MailboxSend` effects committed.
    pub mailbox_sends_committed: usize,
    /// Number of `SignalUpdate` effects committed.
    pub signal_updates_committed: usize,
    /// Number of `MailboxReceiveAttempt` effects that delivered a message.
    pub mailbox_receives_committed: usize,
    /// Number of `MailboxReceiveAttempt` effects that blocked on an empty mailbox.
    pub mailbox_receives_blocked: usize,
    /// Number of `DmaEnqueue` effects committed onto the DMA queue.
    pub dma_enqueued: usize,
    /// Number of `WakeUnit` effects committed.
    pub wakes_committed: usize,
    /// Number of `WaitOnEvent` effects committed.
    pub waits_committed: usize,
    /// DMA completions that fired at this boundary. Always zero from
    /// [`CommitPipeline::process`]; the runtime fills it during `commit_step`.
    pub dma_completions_fired: usize,
    /// `ReservationAcquire`s that installed or replaced an entry.
    ///
    /// A second acquire on the same unit bumps this counter AND
    /// `reservations_cleared` while leaving the table size unchanged;
    /// tooling that wants net installs must reconcile both.
    pub reservation_acquires_committed: usize,
    /// Number of `ConditionalStore` effects committed.
    pub conditional_stores_committed: usize,
    /// `ConditionalStore`s that reached apply without a prior reservation
    /// for the emitter. Non-zero indicates an emitter-side LL/SC pre-check
    /// skip or ordering bug.
    pub conditional_stores_without_prior_reservation: usize,
    /// Reservation-table entries dropped during this commit.
    ///
    /// Sources: `SharedWriteIntent` clear-sweep, `ConditionalStore`
    /// emitter-entry drop and cross-unit sweep, and `ReservationAcquire`
    /// clobbers of prior entries on the same unit.
    pub reservations_cleared: usize,
    /// Effects the validation pass staged nothing for: `FaultRaised`,
    /// `TraceMarker`, `RsxFlipRequest`, `SharedReadIntent` and
    /// `ClockRead`.
    ///
    /// The apply pass still acts on `RsxFlipRequest`.
    pub effects_deferred: usize,
    /// `true` if the step faulted and the whole batch was discarded.
    pub fault_discarded: bool,
    /// Equals `effects.len()` when `fault_discarded`, else zero;
    /// per-kind detail lives in the trace stream.
    pub effects_discarded_on_fault: usize,
    /// Units transitioned to `Blocked` during this commit, with the reason.
    pub blocked_units: Vec<(UnitId, BlockReason)>,
    /// Excludes DMA-completion wakes.
    pub woken_units: Vec<UnitId>,
}

/// Why the commit pipeline blocked a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// `MailboxReceiveAttempt` on an empty mailbox.
    MailboxEmpty,
    /// `WaitOnEvent` effect.
    WaitOnEvent,
    /// SPU `MFC_RD_TAG_STAT` yielded with the masked tags not yet
    /// complete; runtime parks the unit until a DMA completion
    /// publishes the missing tag bit and wakes the issuer.
    DmaWait,
}

/// Mutable references to every subsystem the commit pipeline touches.
pub struct CommitContext<'a> {
    /// Guest memory the staged writes drain into.
    pub memory: &'a mut GuestMemory,
    /// Space 0's memory, `Some` only where [`Self::memory`] is a child
    /// space and `None` where the two are the same memory.
    ///
    /// Every DMA transfer reads and writes space 0, whatever space its
    /// issuer runs in. A caller that validates a DMA range resolves it
    /// against this field. Against [`Self::memory`] the check reads
    /// bytes the transfer never touches.
    pub dma_memory: Option<&'a GuestMemory>,
    /// Unit registry queried for source/target validation and status overrides.
    pub units: &'a mut UnitRegistry,
    /// Mailbox registry for send and receive-attempt effects.
    pub mailboxes: &'a mut MailboxRegistry,
    /// Signal registry for signal-update effects.
    pub signals: &'a mut SignalRegistry,
    /// DMA queue that enqueued completions are appended to.
    pub dma_queue: &'a mut DmaQueue,
    /// Latency model used to compute DMA completion times.
    pub dma_latency: &'a dyn DmaLatencyModel,
    /// Current guest time used as the base for DMA latency calculations.
    pub now: GuestTicks,
    /// Reservation table mutated by LL/SC and clear-sweep paths.
    pub reservations: &'a mut ReservationTable,
    /// Zero means GCM has not been initialised; `RsxLabelWrite` commits as
    /// a 4-byte big-endian store at `rsx_label_base + offset`.
    ///
    /// The value also decides whether an offset is checked against the
    /// RSX label area's extent: relative offsets are only meaningful once
    /// a base exists, so the guard is skipped while this is zero.
    pub rsx_label_base: u32,
    /// Write-only from this pipeline.
    pub rsx_flip: &'a mut crate::rsx::flip::RsxFlipState,
    /// Count of `RsxLabelWrite` effects that reach the label-area check.
    ///
    /// `process()` increments it beside that `debug_assert!`, so in a
    /// debug build the count equals the number of times the assert ran.
    pub rsx_label_writes_committed: &'a mut u64,
    /// The debug observer that [`CommitPipeline::process`] reports each
    /// drained write to.
    pub tap: Option<&'a mut (dyn crate::runtime::RuntimeTap + 'static)>,
}

/// The commit pipeline.
#[derive(Debug, Default, Clone)]
pub struct CommitPipeline {}

impl CommitPipeline {
    /// Construct an empty commit pipeline.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process the effects produced by a single unit step.
    ///
    /// Validation runs over every effect first; staged memory writes
    /// drain as one atomic operation before any other subsystem is
    /// mutated. See the module docs for the full contract.
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

        // The IIFE channels validation failures through `staging.clear()`
        // before propagating; `StagingMemory`'s Drop debug-asserts the
        // buffer is empty at release.
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
                                source_unit: *source,
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
                    Effect::DmaEnqueue { request, payload } => {
                        if request.direction() != DmaDirection::Put {
                            ctx.units
                                .set_status_override(request.issuer(), UnitStatus::Faulted);
                            return Err(CommitError::DmaDirectionUnsupported { effect_index: idx });
                        }
                        // An inline payload is the bytes, so the source
                        // is never read and needs no range that
                        // resolves. What the payload does need is the
                        // destination's length: the completion writes it
                        // over that whole range.
                        if let Some(bytes) = payload {
                            if bytes.len() as u64 != request.destination().length() {
                                ctx.units
                                    .set_status_override(request.issuer(), UnitStatus::Faulted);
                                return Err(CommitError::DmaPayloadLengthMismatch {
                                    effect_index: idx,
                                });
                            }
                        } else {
                            let src = request.source();
                            // Unlogged: the transfer's own read at
                            // completion is the one the runtime reports,
                            // and this batch may still be refused below.
                            let dma_mem: &GuestMemory = ctx.dma_memory.unwrap_or(&*ctx.memory);
                            let resolves: Result<(), MemError> =
                                dma_mem.with_reads_unlogged(|mem: &GuestMemory| {
                                    mem.read_checked(src).map(|_| ())
                                });
                            if let Err(err) = resolves {
                                ctx.units
                                    .set_status_override(request.issuer(), UnitStatus::Faulted);
                                return Err(match err {
                                    MemError::Unmapped(_) => {
                                        CommitError::DmaSourceOutOfRange { effect_index: idx }
                                    }
                                    other => CommitError::Memory(other),
                                });
                            }
                        }
                        let dst = request.destination();
                        // The completion writes the destination in
                        // space 0 too, whatever space the issuer runs
                        // in.
                        let dst_mem: &GuestMemory = ctx.dma_memory.unwrap_or(&*ctx.memory);
                        if let Err(err) = dst_mem.validate_write(dst, dst.length() as usize) {
                            // Marking the issuer Faulted prevents the SPU
                            // from polling MFC_RD_TAG_STAT for a tag bit
                            // that never arrives and time-warping to an
                            // empty queue -> StepError::AllBlocked.
                            ctx.units
                                .set_status_override(request.issuer(), UnitStatus::Faulted);
                            return Err(match err {
                                MemError::Unmapped(_) | MemError::LengthMismatch => {
                                    CommitError::DmaDestinationOutOfRange { effect_index: idx }
                                }
                                MemError::ReservedWrite { addr, region } => {
                                    CommitError::DmaDestinationReserved {
                                        effect_index: idx,
                                        addr,
                                        region,
                                    }
                                }
                                other => CommitError::Memory(other),
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
                                source_unit: *source,
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
                                source_unit: *source,
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
                                source_unit: *source,
                            });
                        }
                    }
                    Effect::RsxLabelWrite { offset, value } => {
                        // Semaphore, notify and report slots all resolve
                        // against one base, so a correct guest stays inside
                        // the whole area. With a zero base, `offset` is
                        // absolute.
                        debug_assert!(
                            ctx.rsx_label_base == 0
                                || (*offset as usize) < cellgov_ps3_abi::lv2::rsx::reports::SIZE,
                            "RsxLabelWrite offset {:#x} escapes the {:#x}-byte RSX label area \
                         under label base {:#x} (guest bug? semaphores 0..0x1000, notify at \
                         0x1000, reports at 0x1400)",
                            *offset,
                            cellgov_ps3_abi::lv2::rsx::reports::SIZE,
                            ctx.rsx_label_base,
                        );
                        *ctx.rsx_label_writes_committed =
                            ctx.rsx_label_writes_committed.wrapping_add(1);
                        // Two u32s widened to u64: the sum cannot wrap.
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
                    Effect::FaultRaised { kind, source } => {
                        // The variant's contract is that the whole step is
                        // discarded, but this pipeline drives the discard
                        // off `YieldReason::Fault`, which already returned
                        // above. Reaching here means the emitter raised a
                        // fault without yielding one, and every sibling
                        // effect in the batch is about to commit.
                        debug_assert!(
                            result.yield_reason == YieldReason::Fault,
                            "FaultRaised({kind:?}) from unit {} in a batch that yielded {:?}; \
                         the discard contract is driven by the yield reason, so the batch \
                         commits instead",
                            source.raw(),
                            result.yield_reason,
                        );
                        deferred += 1;
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

        // The drain is the only fallible op in the apply pass; a
        // new fallible op below would need rollback machinery to
        // preserve the atomic-batch contract. It validates the whole
        // batch before it applies any write, so the observer sees no
        // write from a refused batch.
        //
        // The drain leaves the staging buffer populated on
        // validation failure; clear it so that `StagingMemory`'s Drop
        // guard holds on both the pre_validate and the drain failure
        // paths.
        let tap = &mut ctx.tap;
        let drained = staging.drain_into_observed(ctx.memory, |range, bytes| {
            if let Some(tap) = tap.as_deref_mut() {
                tap.write(range.start().raw(), bytes);
            }
        });
        if let Err(e) = drained {
            staging.clear();
            return Err(CommitError::Memory(e));
        }
        for effect in effects {
            match effect {
                Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    // force_send: PPE-overrun semantics
                    // [CBE-Handbook p:541 s:19.6.6.2].
                    ctx.mailboxes
                        .get_mut(*mailbox)
                        .expect("pre-validated mailbox id")
                        .force_send(message.raw());
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
                Effect::SharedWriteIntent { range, source, .. } => {
                    // Emitter's own reservation preserved: PPC invalidates
                    // only on stores from another processor
                    // [PPC-Book2 p:10 s:1.7.3.1].
                    reservations_cleared += ctx.reservations.clear_covering(
                        range.start().raw(),
                        range.length(),
                        Some(*source),
                    );
                }
                Effect::ReservationAcquire { line_addr, source } => {
                    // Canonicalize to 128-byte line at insert; callers may
                    // pass a raw EA. A clobbered prior entry on the same
                    // unit is counted as cleared.
                    let prior = ctx
                        .reservations
                        .insert_or_replace(*source, ReservedLine::containing(*line_addr));
                    if prior.is_some() {
                        reservations_cleared += 1;
                    }
                    reservation_acquires += 1;
                }
                Effect::ConditionalStore { range, source, .. } => {
                    // Drop the emitter's own entry first so the cross-unit
                    // sweep below cannot double-count it. A missing prior
                    // entry here flags an emitter-side bug via
                    // `conditional_stores_without_prior_reservation`.
                    if ctx.reservations.remove_if_present(*source).is_some() {
                        reservations_cleared += 1;
                    } else {
                        conditional_stores_without_prior_reservation += 1;
                    }
                    reservations_cleared +=
                        ctx.reservations
                            .clear_covering(range.start().raw(), range.length(), None);
                }
                Effect::RsxLabelWrite { offset, .. } => {
                    // The RSX is a bus master with no `UnitId`, so no
                    // entry is exempt: every reservation covering the four
                    // bytes it lands on is dropped.
                    // [PPC-Book2 p:10 s:1.7.3.1] a store by some other
                    // mechanism into the granule loses the reservation.
                    let start = (ctx.rsx_label_base as u64).wrapping_add(*offset as u64);
                    reservations_cleared += ctx.reservations.clear_covering(start, 4, None);
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

#[cfg(test)]
#[path = "tests/dma_argument_tests.rs"]
mod dma_argument_tests;
