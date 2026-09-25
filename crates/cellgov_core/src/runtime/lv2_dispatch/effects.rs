//! Applying an LV2 dispatch's effects batch, and the all-or-none validation of its memory writes.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::MemError;
use cellgov_trace::HostWriter;

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::Runtime;

impl Runtime {
    /// Apply an `Lv2Dispatch` effects batch into `space` through
    /// [`Runtime::host_write`], which bypasses the commit pipeline's
    /// [`StagingMemory`].
    ///
    /// The `SharedWriteIntent` subset commits all-or-none: a
    /// validation failure logs `dispatch.lv2_effect_apply_failed` and
    /// lands none of the writes, while non-memory effects still apply.
    /// A requested flip transitions on the next `commit_step`
    /// boundary. Variants with no LV2 producer are dropped with a
    /// named invariant break.
    ///
    /// These mutations do not participate in atomic-batch
    /// discard-on-fault: the syscall has already returned its result
    /// to the guest by the time the containing batch finalizes, so
    /// syscall-side state persists even when the batch's unit-staged
    /// effects are discarded. `Runtime::commit_step` drains staged
    /// unit effects first and calls this function second, so an LV2
    /// write lands after any same-batch unit store to the same range.
    ///
    /// [`StagingMemory`]: cellgov_mem::StagingMemory
    ///
    /// # Cross-module contract
    ///
    /// LV2 handlers must not emit a non-memory effect whose semantics
    /// depend on a co-batched `SharedWriteIntent` having committed;
    /// the `debug_assert!` below traps that condition. `space` is the
    /// syscall caller's space for dispatch-time effects and the
    /// expiring waiter's space for `expire_wait` effects, so a handler
    /// must emit only intents whose pointers were decoded from that
    /// unit's own syscall; waiter-side payloads belong on
    /// `PendingResponse` / `response_updates`.
    pub(in crate::runtime) fn apply_lv2_effects(
        &mut self,
        effects: &[Effect],
        space: AddressSpaceId,
    ) {
        let memory_failure = self.validate_lv2_memory_subset(effects, space);
        debug_assert!(
            !(memory_failure.is_some()
                && effects
                    .iter()
                    .any(|e| !matches!(e, Effect::SharedWriteIntent { .. }))),
            "LV2 handler co-emitted a SharedWriteIntent that failed validation alongside \
             a non-memory effect; non-memory effects land unconditionally, leaving the \
             batch in a forbidden partial state. failure={:?}",
            memory_failure,
        );
        if let Some((range, err)) = memory_failure {
            let addr = range.start().raw();
            let length = range.length();
            self.lv2_host.log_invariant_break(
                "dispatch.lv2_effect_apply_failed",
                format_args!(
                    "LV2 SharedWriteIntent validation failed at addr=0x{addr:016x} \
                     length={length} in space {}: {err}; memory subset rolled back, \
                     non-memory effects still applied",
                    space.raw(),
                ),
            );
        }
        for effect in effects {
            let site = match effect {
                Effect::SharedWriteIntent {
                    range,
                    bytes,
                    source,
                    ..
                } => {
                    if memory_failure.is_some() {
                        continue;
                    }
                    // The reservation sweep exempts `source`, so an
                    // intent whose source lives in another space would
                    // spare a unit that holds nothing here. The real
                    // holder then keeps a stale reservation.
                    debug_assert_eq!(
                        self.spaces.space_of(*source),
                        space,
                        "LV2 SharedWriteIntent at {:#x} from unit {} targets space {}, \
                         which is not that unit's; waiter-side payloads belong on \
                         PendingResponse",
                        range.start().raw(),
                        source.raw(),
                        space.raw(),
                    );
                    // LV2 direct commits bypass the commit pipeline's
                    // shared-view fanout; a write landing inside a
                    // shared view would leave sibling views incoherent.
                    // No LV2 emitter targets shared segments yet --
                    // surface the first one here instead of as silent
                    // incoherence (same guard as ConditionalStore in
                    // fanout_shared_writes). The invariant break keeps
                    // the witness loud in release builds, where the
                    // debug_assert compiles out and the write would
                    // otherwise land in this view alone.
                    if self.range_intersects_shared_view(space, *range) {
                        self.lv2_host.log_invariant_break(
                            "dispatch.lv2_write_targets_shared_view",
                            format_args!(
                                "LV2 SharedWriteIntent at 0x{:x}+0x{:x} targets a shared \
                                 view in space {}; cross-space replication of LV2 direct \
                                 commits is not modeled, sibling views are now incoherent",
                                range.start().raw(),
                                range.length(),
                                space.raw(),
                            ),
                        );
                        debug_assert!(
                            false,
                            "LV2 SharedWriteIntent at {:#x}+{:#x} targets a shared view in \
                             space {}; cross-space replication of LV2 direct commits is not \
                             modeled",
                            range.start().raw(),
                            range.length(),
                            space.raw(),
                        );
                    }
                    self.host_write(
                        HostWriter::Lv2Effect,
                        space,
                        *range,
                        bytes.bytes(),
                        Some(*source),
                    )
                    .expect(
                        "validate_lv2_memory_subset called GuestMemory::validate_write -- \
                         the same predicate apply_commit uses internally -- so this Err \
                         path is structurally unreachable",
                    );
                    self.lv2_direct_committed_writes =
                        self.lv2_direct_committed_writes.wrapping_add(1);
                    self.last_lv2_effects.push(effect.clone());
                    continue;
                }
                Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    if let Some(mut mbox) = self.mailbox_registry.get_mut(*mailbox) {
                        // [CBE-Handbook p:541 s:19.6.6.2] outbound
                        // write-blocking path is not wired here yet.
                        mbox.force_send(message.raw());
                    } else {
                        // `handle_register_spu` mints the mailbox
                        // beside the unit, so a miss here names a
                        // host-side disagreement between the thread
                        // table and the mailbox registry.
                        self.lv2_host.log_invariant_break(
                            "runtime.apply_lv2_effects_mailbox_send_unregistered",
                            format_args!(
                                "LV2 dispatch sent to mailbox {} with no registry entry; \
                                 the message is discarded and the target's next receive \
                                 reads the state before it",
                                mailbox.raw(),
                            ),
                        );
                    }
                    let target = UnitId::new(mailbox.raw());
                    if self.registry.effective_status(target) == Some(UnitStatus::Blocked) {
                        self.registry
                            .set_status_override(target, UnitStatus::Runnable);
                    }
                    self.last_lv2_effects.push(effect.clone());
                    continue;
                }
                Effect::RsxFlipRequest { buffer_index } => {
                    self.rsx_flip.request_flip(*buffer_index);
                    self.last_lv2_effects.push(effect.clone());
                    continue;
                }
                // Execution units and the FIFO advance pass emit these
                // variants, and no LV2 handler does. A boot prints the
                // detail line of its first break alone, so each variant
                // logs under its own site and the per-site count
                // identifies it.
                Effect::MailboxReceiveAttempt { .. } => {
                    "runtime.apply_lv2_effects_unsupported_mailbox_receive_attempt"
                }
                Effect::DmaEnqueue { .. } => "runtime.apply_lv2_effects_unsupported_dma_enqueue",
                Effect::WaitOnEvent { .. } => "runtime.apply_lv2_effects_unsupported_wait_on_event",
                Effect::WakeUnit { .. } => "runtime.apply_lv2_effects_unsupported_wake_unit",
                Effect::SignalUpdate { .. } => {
                    "runtime.apply_lv2_effects_unsupported_signal_update"
                }
                Effect::FaultRaised { .. } => "runtime.apply_lv2_effects_unsupported_fault_raised",
                Effect::TraceMarker { .. } => "runtime.apply_lv2_effects_unsupported_trace_marker",
                Effect::ReservationAcquire { .. } => {
                    "runtime.apply_lv2_effects_unsupported_reservation_acquire"
                }
                Effect::ConditionalStore { .. } => {
                    "runtime.apply_lv2_effects_unsupported_conditional_store"
                }
                Effect::RsxLabelWrite { .. } => {
                    "runtime.apply_lv2_effects_unsupported_rsx_label_write"
                }
                Effect::SharedReadIntent { .. } => {
                    "runtime.apply_lv2_effects_unsupported_shared_read_intent"
                }
                Effect::ClockRead { .. } => "runtime.apply_lv2_effects_unsupported_clock_read",
            };
            self.lv2_host.log_invariant_break(
                site,
                format_args!(
                    "LV2 dispatch emitted {effect:?}, which no LV2 handler produces; \
                     effect dropped"
                ),
            );
        }
    }

    /// Pre-validate the `SharedWriteIntent` subset of `effects`
    /// against `space`'s memory via `GuestMemory::validate_write`
    /// (the predicate `apply_commit` itself calls). Returns the first
    /// failing intent or `None`.
    fn validate_lv2_memory_subset(
        &self,
        effects: &[Effect],
        space: AddressSpaceId,
    ) -> Option<(cellgov_mem::ByteRange, MemError)> {
        let mem = crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        for effect in effects {
            if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                if let Err(err) = mem.validate_write(*range, bytes.len()) {
                    return Some((*range, err));
                }
            }
        }
        None
    }
}
