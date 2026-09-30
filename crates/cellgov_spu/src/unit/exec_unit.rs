//! The [`ExecutionUnit`] contract the runtime drives the SPU through,
//! with the fetch-decode-execute loop in `run_until_yield`.

use super::spu_unit::{SpuExecutionUnit, SpuSnapshot};
use super::transfer::{copy_into_local_store, shared_read};
use crate::exec::{SpuFault, SpuStepOutcome};
use crate::fault_codes::{
    guest_fault, guest_fault_for, FAULT_LS_OUT_OF_RANGE, FAULT_UNIMPLEMENTED_INSN,
};
use crate::instruction::SpuDecodeError;
use crate::stop::SpuStopKind;
use crate::{decode, exec};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    BarrierKind, ChannelStall, ExecutionContext, ExecutionStepResult, ExecutionUnit,
    LocalDiagnostics, ProblemStateError, RestartError, SignalNotifier, StallWake, StopRegisters,
    UnitStatus, YieldReason,
};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::{MFC_ATOMIC_STAT_G, SPU_STATUS_R};
use cellgov_ps3_abi::hw::spu_isa;
use cellgov_time::{Budget, InstructionCost};

impl ExecutionUnit for SpuExecutionUnit {
    type Snapshot = SpuSnapshot;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        self.status
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        // A woken stall runs its channel access again from the PC it
        // left, so nothing of the old park carries over.
        self.stall = None;

        // A group reads complete when this unit has no outstanding
        // transfer with its tag, so a reused tag reads incomplete until its
        // new transfer lands.
        // [CBEA p:128 s:9.3.6] a set bit means the group has no outstanding operations.
        // A list with elements still to queue holds its tag group too.
        let channels = &mut self.state.channels;
        let list_tags = channels
            .lists
            .iter()
            .fold(0, |tags, list| tags | list.tag.status_bit());
        channels.tag_status = !(ctx.outstanding_dma_tags() | list_tags);
        // Each queued command holds a slot until it completes, and a list
        // holds its one slot until it queues its last element. A count
        // that rises from 0 here is the Qv event's edge.
        let held = u32::try_from(channels.lists.len()).unwrap_or(u32::MAX);
        channels.cmd_queue_free = spu::MFC_SPU_QUEUE_DEPTH
            .saturating_sub(ctx.dma_queue_occupancy())
            .saturating_sub(held);
        // A list stalls once its stall-and-notify element leaves the queue.
        // [CBEA p:129 s:9.3.7] the stall occurs after the flagged element's transfer completes, and sets the group's bit in MFC_RdListStallStat.
        for list in channels.lists.iter_mut().filter(|list| !list.stalled) {
            let bit = list.tag.status_bit();
            if ctx.list_stall_tags() & bit == 0 {
                list.stalled = true;
                channels.list_stall_status |= bit;
            }
        }
        self.state.channels.settle_tag_update();
        self.state.channels.in_mbox = ctx.inbound_mailbox().to_vec();
        // A multisource synchronization request completes once the queue
        // holds none of the transfers it tracks. A request this step makes
        // tracks the transfers the queue holds now.
        // [CBEA p:143 s:9.10] the count returns to 1 when the tracked transfers complete.
        let oldest = ctx.oldest_mfc_transfer();
        let channels = &mut self.state.channels;
        if channels
            .mssync_tracking
            .is_some_and(|upto| oldest.is_none_or(|seq| seq >= upto))
        {
            channels.mssync_tracking = None;
        }
        channels.mssync_horizon = oldest.map(|_| ctx.dma_next_sequence());
        // Every source count the step entry refreshed can have risen.
        self.state.update_events();

        // Mirror cross-unit reservation invalidation. The context view is
        // frozen for the step, so a single entry-time check suffices.
        // Only another unit's store drops the committed entry while the
        // local register holds a line: the unit's own actions clear the
        // register as they run.
        // [CBEA p:148 s:9.11.1] Lr is set when a snoop external to the MFC resets the reservation, and never for a local action.
        if self.state.reservation().is_some() && !ctx.reservation_held(self.id) {
            self.state.set_reservation(None);
            self.state.raise_events(spu::event::LR);
        }

        let mut remaining = budget.raw();
        effects.clear();

        loop {
            // [SPU-ISA p:251 s:12.1] an interrupt lands between instructions.
            if self.state.interrupt_pending() {
                self.state.take_interrupt();
            }
            let step_pc = self.state.pc as u64;
            let raw = match self.state.fetch() {
                Some(w) => w,
                None => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(guest_fault(FAULT_LS_OUT_OF_RANGE, self.state.pc)),
                        syscall_args: None,
                    };
                }
            };

            let insn = match decode::decode(raw) {
                Ok(i) => i,
                // [CBEA p:33 s:2.1.2] an SPU that meets an invalid instruction halts and records the event in its status register.
                // [CBEA p:93 s:8.5.2] I: invalid instruction detected, SPU stopped imprecisely.
                // [CBEA p:34 s:2.2.3] an optional instruction the implementation lacks is an illegal instruction.
                Err(SpuDecodeError::Unassigned(_) | SpuDecodeError::AbsentOnCbe { .. }) => {
                    self.state.record_stop(SpuStopKind::InvalidInstruction, 0);
                    self.status = UnitStatus::Finished;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Finished,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                Err(SpuDecodeError::Unimplemented { .. }) => {
                    let row = spu_isa::row_for(raw).map_or(0, |(index, _)| index as u32);
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(guest_fault(FAULT_UNIMPLEMENTED_INSN, row)),
                        syscall_args: None,
                    };
                }
            };

            let outcome = exec::execute(&insn, &mut self.state, self.id);
            // A channel access is the one instruction that moves an event
            // source's count within a step.
            if matches!(
                insn,
                crate::instruction::SpuInstruction::Rdch { .. }
                    | crate::instruction::SpuInstruction::Wrch { .. }
            ) {
                self.state.update_events();
            }
            match outcome {
                SpuStepOutcome::Continue => {
                    if ctx.trace_per_step() {
                        if let Some(kind) = barrier_kind(&insn) {
                            self.barriers
                                .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
                        }
                    }
                    self.state.advance_pc();
                }
                SpuStepOutcome::Branch => {}
                SpuStepOutcome::Stop { kind, signal } => {
                    self.state.record_stop(kind, signal);
                    self.status = UnitStatus::Finished;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Finished,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                SpuStepOutcome::Yield {
                    effects: step_effects,
                    reason,
                } => {
                    if ctx.trace_per_step() {
                        if let Some(kind) = queued_mfc_barrier(&insn, &self.state, &step_effects) {
                            self.barriers
                                .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
                        }
                    }
                    effects.extend(step_effects);
                    if reason == YieldReason::ChannelStall {
                        // The access did not retire: PC stays on it, and
                        // the unit names the channel for its waker. Any
                        // enabled event can interrupt the wait, so every
                        // producer ends it.
                        // [CBE-Handbook p:447 s:17.1.6] a blocked access stalls until the channel changes or the SPU is interrupted.
                        self.stall = channel_stall(&insn).map(|stall| ChannelStall {
                            wake: if self.state.interruptible() {
                                StallWake::Event
                            } else {
                                stall.wake
                            },
                            ..stall
                        });
                    } else {
                        self.state.advance_pc();
                    }
                    return ExecutionStepResult {
                        yield_reason: reason,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                SpuStepOutcome::MemoryRead {
                    ea,
                    lsa,
                    size,
                    acquire_line,
                } => {
                    // `ea` comes from MFC_EAH and MFC_EAL, so the guest can
                    // name an address no region backs.
                    // [CBEA p:111 s:9 SPU Channel Map] MFC_EAL is a write channel carrying the low-order SPU effective-address command parameter.
                    let read = copy_into_local_store(&mut self.state, ctx.memory(), ea, lsa, size);
                    if read.is_err() {
                        // The line does not translate: the MFC raises the
                        // data-storage exception for the getllar, which
                        // moves no line and never reports its status.
                        // [CBEA p:118 s:9.1.6] a mapping fault suspends the queue and raises the MFC data-storage interrupt.
                        // The command acquires no reservation over a
                        // line it never read. One an earlier getllar took
                        // stands, in the register as in the committed
                        // table that getllar's acquire reaches.
                        // The record names the line the command moves.
                        let params = cellgov_dma::MfcParameters {
                            lsa,
                            eah: (ea >> 32) as u32,
                            eal: ea as u32,
                            size,
                            tag: self.state.channels.mfc_tag_id,
                        };
                        effects.push(crate::exec::invalid_command(
                            spu::MFC_GETLLAR,
                            params,
                            cellgov_dma::MfcCommandError::DataStorage { ea },
                            &mut self.state,
                            self.id,
                        ));
                        self.state.advance_pc();
                        return ExecutionStepResult {
                            yield_reason: YieldReason::DmaSubmitted,
                            consumed_cost: InstructionCost::new(budget.raw() - remaining),
                            local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                            fault: None,
                            syscall_args: None,
                        };
                    }
                    effects.extend(shared_read(ea, size, self.id));
                    if let Some(line_addr) = acquire_line {
                        // [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] the channel holds the status of the last completed immediate atomic command.
                        self.state.channels.atomic_status = MFC_ATOMIC_STAT_G;
                        self.state.channels.atomic_status_ready = true;
                        self.state
                            .set_reservation(Some(cellgov_sync::ReservedLine::containing(
                                line_addr,
                            )));
                        effects.push(Effect::ReservationAcquire {
                            line_addr,
                            source: self.id,
                        });
                    }
                    self.state.advance_pc();
                }
                SpuStepOutcome::Fault(f) => {
                    self.status = UnitStatus::Faulted;
                    let local_diagnostics = match f {
                        SpuFault::LsOutOfRange(addr) => {
                            LocalDiagnostics::with_pc_ea(step_pc, u64::from(addr))
                        }
                        _ => LocalDiagnostics::with_pc(step_pc),
                    };
                    let fault = guest_fault_for(f);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics,
                        fault: Some(fault),
                        syscall_args: None,
                    };
                }
            }

            debug_assert!(
                self.state.hash_is_current(),
                "state-hash accumulator out of date after retirement at 0x{step_pc:x}"
            );
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                return ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                    fault: None,
                    syscall_args: None,
                };
            }
        }
    }

    fn snapshot(&self) -> SpuSnapshot {
        // Every field is named, so a new one fails to compile here until
        // the snapshot takes it or says why not.
        let SpuExecutionUnit {
            // The unit's identity, not its context.
            id: _,
            state,
            status,
            stall,
            // Trace output the runtime drains, not state.
            barriers: _,
        } = self;
        SpuSnapshot {
            state: state.clone(),
            status: *status,
            stall: *stall,
        }
    }

    fn stop_registers(&self) -> Option<StopRegisters> {
        self.state.stop.map(|stop| StopRegisters {
            status: stop.status_word(),
            npc: stop.npc | u32::from(stop.interrupts_enabled),
        })
    }

    /// [CBEA p:95 s:8.5.3] a restart resumes at SPU_NPC; [CBEA p:94 s:8.5.2] it clears the C, I, S, H and P bits.
    /// [CBEA p:96 s:8.5.3] `SPU_NPC[IE]` is the interrupt-enable state at start.
    fn restart(&mut self) -> Result<(), RestartError> {
        let stop = self.state.stop.take().ok_or(RestartError::NotStopped)?;
        self.state.pc = stop.npc;
        self.state.set_interrupts_enabled(stop.interrupts_enabled);
        self.status = UnitStatus::Runnable;
        Ok(())
    }

    /// [CBEA p:94 s:8.5.2] R is set while the SPU runs and clear once it stops.
    /// A unit CellGov refused issues no instructions, so it reports R clear
    /// with no stop cause.
    fn spu_status(&self) -> Option<u32> {
        if self.status == UnitStatus::Faulted {
            return Some(0);
        }
        Some(
            self.state
                .stop
                .map_or(SPU_STATUS_R, |stop| stop.status_word()),
        )
    }

    /// [CBEA p:92 s:8.5.1] a stop request stops instruction issue; [CBEA p:95 s:8.5.3] SPU_NPC then names the next instruction.
    /// A unit CellGov refused keeps its refusal, and a stopped unit its
    /// stop.
    fn request_stop(&mut self, waiting: bool) -> Result<(), ProblemStateError> {
        if self.state.stop.is_none() && self.status != UnitStatus::Faulted {
            self.state
                .record_stop(SpuStopKind::Requested { waiting }, 0);
            self.status = UnitStatus::Finished;
            self.stall = None;
        }
        Ok(())
    }

    /// [CBEA p:95 s:8.5.3] a write updates SPU_NPC only while the SPU is stopped.
    /// [CBEA p:96 s:8.5.3] its least significant bit is the interrupt-enable state at start.
    /// A new SPU_NPC abandons a parked channel access, which took
    /// nothing.
    fn write_npc(&mut self, npc: u32) -> Result<(), ProblemStateError> {
        if self.status == UnitStatus::Faulted {
            return Err(ProblemStateError::Refused);
        }
        let lslr = self.state.lslr();
        let stop = self.state.stop.as_mut().ok_or(ProblemStateError::Running)?;
        stop.npc = npc & lslr & !3;
        stop.interrupts_enabled = npc & 1 != 0;
        Ok(())
    }

    fn write_signal(
        &mut self,
        register: SignalNotifier,
        value: u32,
    ) -> Result<(), ProblemStateError> {
        let index = match register {
            SignalNotifier::One => 0,
            SignalNotifier::Two => 1,
        };
        self.state.signals[index].write(value);
        Ok(())
    }

    /// [CBEA p:239 s:16.4] SPU_Cfg sets each signal-notification register to overwrite or to OR.
    fn set_signal_logical_or(
        &mut self,
        register: SignalNotifier,
        logical_or: bool,
    ) -> Result<(), ProblemStateError> {
        let index = match register {
            SignalNotifier::One => 0,
            SignalNotifier::Two => 1,
        };
        self.state.signals[index].mode = if logical_or {
            crate::state::SignalNotifyMode::LogicalOr
        } else {
            crate::state::SignalNotifyMode::Overwrite
        };
        Ok(())
    }

    /// [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox takes the oldest message out of the queue.
    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.state.channels.out_mbox.take())
    }

    fn channel_stall(&self) -> Option<ChannelStall> {
        self.stall
    }

    fn local_reservation(&self) -> Option<u64> {
        self.state.reservation().map(|line| line.addr())
    }

    fn drain_barriers(&mut self) -> Vec<cellgov_exec::RetiredBarrier> {
        std::mem::take(&mut self.barriers)
    }

    /// [CBEA p:60 s:7.5] a get moves main-storage bytes into local storage.
    ///
    /// Each byte's address wraps by the limit register, so a landing never
    /// escapes local store.
    fn land_local_store(&mut self, lsa: u32, bytes: &[u8]) -> Result<(), ProblemStateError> {
        self.state.write_ls_wrapped(lsa, bytes);
        Ok(())
    }

    /// Each byte's address wraps by the limit register, as for a landing.
    fn read_local_store(&self, lsa: u32, len: u32) -> Result<Vec<u8>, ProblemStateError> {
        Ok(self.state.read_ls_wrapped(lsa, len))
    }

    fn local_memory_hash(&self) -> Option<u64> {
        Some(super::spu_unit::local_store_hash(&self.state.ls))
    }
}

/// The barrier `insn` is, if it is one.
fn barrier_kind(insn: &crate::instruction::SpuInstruction) -> Option<BarrierKind> {
    use crate::instruction::SpuInstruction;
    match insn {
        SpuInstruction::Sync { c: false } => Some(BarrierKind::SpuSync),
        SpuInstruction::Sync { c: true } => Some(BarrierKind::SpuSyncC),
        SpuInstruction::Dsync => Some(BarrierKind::SpuDsync),
        _ => None,
    }
}

/// The barrier an `MFC_Cmd` write queued, if its command orders the
/// queue. A write whose first effect is an invalid command orders
/// nothing.
fn queued_mfc_barrier(
    insn: &crate::instruction::SpuInstruction,
    state: &crate::state::SpuState,
    effects: &[Effect],
) -> Option<BarrierKind> {
    let crate::instruction::SpuInstruction::Wrch {
        channel: spu::MFC_CMD,
        rt,
    } = *insn
    else {
        return None;
    };
    if !matches!(effects.first(), Some(Effect::DmaEnqueue { .. })) {
        return None;
    }
    exec::mfc_barrier_kind(state.reg_word(rt))
}

/// The channel a stalled instruction names and the event that ends the
/// stall, or `None` for an instruction with no blocking channel.
fn channel_stall(insn: &crate::instruction::SpuInstruction) -> Option<ChannelStall> {
    use crate::instruction::SpuInstruction;
    let channel = match insn {
        SpuInstruction::Rdch { channel, .. } | SpuInstruction::Wrch { channel, .. } => *channel,
        _ => return None,
    };
    let wake = match channel {
        spu::SPU_RD_IN_MBOX => StallWake::MailboxDelivery,
        spu::MFC_RD_TAG_STAT | spu::MFC_RD_LIST_STALL_STAT => StallWake::DmaCompletion,
        spu::SPU_WR_OUT_MBOX => StallWake::OutboundMailboxRead,
        spu::MFC_CMD => StallWake::CommandQueueSlot,
        spu::SPU_RD_SIG_NOTIFY_1 => StallWake::SignalWrite(SignalNotifier::One),
        spu::SPU_RD_SIG_NOTIFY_2 => StallWake::SignalWrite(SignalNotifier::Two),
        spu::MFC_RD_ATOMIC_STAT => StallWake::AtomicCommandCompletion,
        spu::MFC_WR_MSSYNC_REQ => StallWake::MultisourceSync,
        spu::SPU_RD_EVENT_STAT => StallWake::Event,
        _ => return None,
    };
    Some(ChannelStall { channel, wake })
}
