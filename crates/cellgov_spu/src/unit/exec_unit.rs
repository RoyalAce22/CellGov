//! The [`ExecutionUnit`] contract the runtime drives the SPU through,
//! with the fetch-decode-execute loop in `run_until_yield`.

use super::spu_unit::{SpuExecutionUnit, SpuSnapshot};
use super::step;
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
    LocalDiagnostics, OutMailboxes, ProblemStateError, RestartError, SignalNotifier, StallWake,
    StopRegisters, UnitStatus, YieldReason,
};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::SPU_STATUS_R;
use cellgov_ps3_abi::hw::spu_isa;
use cellgov_ps3_abi::hw::spu_issue::SPU_GETLLAR_CYCLES;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

/// The guest tick at which the unit's cycle count reaches `cycles`.
///
/// The step starts at `start`, when the unit's cycle count is `base`
/// ticks.
fn tick_at(start: GuestTicks, base: u64, cycles: u64) -> GuestTicks {
    GuestTicks::new(
        start
            .raw()
            .saturating_add(cellgov_time::spu_cycles_to_ticks(cycles).saturating_sub(base)),
    )
}

impl SpuExecutionUnit {
    /// The guest ticks the step takes, from its start to now.
    ///
    /// `base` is the unit's cycle count, in ticks, when the step began.
    fn cost(&self, base: u64) -> InstructionCost {
        InstructionCost::new(
            cellgov_time::spu_cycles_to_ticks(self.pipeline.elapsed()).saturating_sub(base),
        )
    }
}

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
        // left, so the unit drops the stall and resumes the pipeline
        // after the park.
        if self.stall.take().is_some() {
            self.pipeline.resume_after_park();
        }

        step::enter_step(&mut self.state, self.id, ctx);

        // The budget bounds the instructions the step retires; the step
        // costs the guest ticks its cycles take. The step reads the ticks
        // from the unit's whole run of cycles, so a step that ends inside
        // a tick leaves the rest of it to the next step.
        let mut remaining = budget.raw();
        let start = ctx.current_tick();
        let base = cellgov_time::spu_cycles_to_ticks(self.pipeline.elapsed());
        effects.clear();

        loop {
            // An interrupt lands at the cycle the next instruction could
            // issue in.
            self.state
                .set_clock(tick_at(start, base, self.pipeline.next_issue()));
            if step::take_pending_interrupt(&mut self.state) {
                // Entering the handler flushes the pipeline as a
                // mispredicted branch to address 0 does. No capture
                // times the entry.
                self.pipeline.redirect(self.state.pc);
            }
            let step_pc = self.state.pc as u64;
            let raw = match self.state.fetch() {
                Some(w) => w,
                None => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: self.cost(base),
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
                        consumed_cost: self.cost(base),
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
                        consumed_cost: self.cost(base),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(guest_fault(FAULT_UNIMPLEMENTED_INSN, row)),
                        syscall_args: None,
                    };
                }
            };

            // The instruction runs at the cycle it issues in.
            let slot = self.pipeline.plan(&self.state, raw, &insn);
            self.state.set_clock(tick_at(start, base, slot.cycle));
            let outcome = step::execute_instruction(&insn, &mut self.state, self.id);
            match outcome {
                SpuStepOutcome::Continue => {
                    if ctx.trace_per_step() {
                        if let Some(kind) = barrier_kind(&insn) {
                            self.barriers
                                .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
                        }
                    }
                    self.pipeline.retire(slot, None);
                    self.state.advance_pc();
                }
                SpuStepOutcome::Branch => self.pipeline.retire(slot, Some(self.state.pc)),
                SpuStepOutcome::Stop { kind, signal } => {
                    self.pipeline.retire(slot, None);
                    self.state.record_stop(kind, signal);
                    self.retire(step_pc, ctx.trace_per_step());
                    self.status = UnitStatus::Finished;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Finished,
                        consumed_cost: self.cost(base),
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
                        self.pipeline.retire(slot, None);
                        self.state.advance_pc();
                        self.retire(step_pc, ctx.trace_per_step());
                    }
                    return ExecutionStepResult {
                        yield_reason: reason,
                        consumed_cost: self.cost(base),
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
                    let completion = step::complete_memory_read(
                        &mut self.state,
                        ctx.memory(),
                        self.id,
                        ea,
                        lsa,
                        size,
                        acquire_line,
                        effects,
                    );
                    self.pipeline.retire(slot, None);
                    if completion == step::MemoryReadCompletion::Refused {
                        self.state.advance_pc();
                        self.retire(step_pc, ctx.trace_per_step());
                        return ExecutionStepResult {
                            yield_reason: YieldReason::DmaSubmitted,
                            consumed_cost: self.cost(base),
                            local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                            fault: None,
                            syscall_args: None,
                        };
                    }
                    if completion == (step::MemoryReadCompletion::Read { acquired: true }) {
                        // The getllar takes the round trip the console
                        // measures.
                        self.pipeline.charge(SPU_GETLLAR_CYCLES);
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
                        consumed_cost: self.cost(base),
                        local_diagnostics,
                        fault: Some(fault),
                        syscall_args: None,
                    };
                }
            }

            self.retire(step_pc, ctx.trace_per_step());
            debug_assert!(
                self.state.hash_is_current(),
                "state-hash accumulator out of date after retirement at 0x{step_pc:x}"
            );
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                return ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_cost: self.cost(base),
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
            pipeline,
            // Trace output the runtime drains, not state.
            barriers: _,
            per_step_hashes: _,
            per_step_full_states: _,
            // Trace instruments: the step the records carry and the
            // window of full records.
            retirement_counter: _,
            full_state_window: _,
        } = self;
        SpuSnapshot {
            state: state.clone(),
            status: *status,
            stall: *stall,
            pipeline: pipeline.clone(),
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
        // [CBE-Handbook p:695 s:24.3.3.4] a hint stays loaded until another hint or a synchronizing event, a sync or a start, replaces it.
        self.pipeline.clear_hints();
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

    fn connect_out_intr_mbox(&mut self) -> Result<(), ProblemStateError> {
        self.state.channels.out_intr_mbox_connected = true;
        Ok(())
    }

    /// [CBEA p:134 s:9.5.2] a read of the SPU Outbound Interrupt Mailbox Register frees the slot a write took.
    fn read_out_intr_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.state.channels.out_intr_mbox.take())
    }

    fn take_out_intr_mbox_interrupt(&mut self) -> bool {
        std::mem::take(&mut self.state.channels.out_intr_mbox_interrupt)
    }

    fn out_mailboxes(&self) -> Option<OutMailboxes> {
        let c = &self.state.channels;
        Some(OutMailboxes {
            out_mbox: c.out_mbox,
            out_intr_mbox: c.out_intr_mbox,
        })
    }

    fn channel_stall(&self) -> Option<ChannelStall> {
        self.stall
    }

    /// The decrementer's zero crossing, for a unit parked where an event
    /// ends its wait.
    fn event_deadline(&self) -> Option<cellgov_time::GuestTicks> {
        self.stall
            .filter(|stall| stall.wake == StallWake::Event)
            .and_then(|_| self.state.decrementer_event_tick())
    }

    fn local_reservation(&self) -> Option<u64> {
        self.state.reservation().map(|line| line.addr())
    }

    fn drain_barriers(&mut self) -> Vec<cellgov_exec::RetiredBarrier> {
        std::mem::take(&mut self.barriers)
    }

    fn drain_retired_spu_state_hashes(&mut self) -> Vec<(u64, u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_spu_state_full(&mut self) -> Vec<(u64, u64, cellgov_exec::SpuFingerprint)> {
        std::mem::take(&mut self.per_step_full_states)
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
        Some(super::spu_unit::local_memory_hash(&self.state))
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
        spu::SPU_WR_OUT_MBOX | spu::SPU_WR_OUT_INTR_MBOX => StallWake::OutboundMailboxRead,
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
