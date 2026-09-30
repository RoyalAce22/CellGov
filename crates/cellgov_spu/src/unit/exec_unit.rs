//! The [`ExecutionUnit`] contract the runtime drives the SPU through,
//! with the fetch-decode-execute loop in `run_until_yield`.

use super::spu_unit::{SpuExecutionUnit, SpuSnapshot};
use super::transfer::{copy_into_local_store, shared_read, CopyRefusal};
use crate::exec::{SpuFault, SpuStepOutcome};
use crate::fault_codes::{
    guest_fault, guest_fault_for, FAULT_LS_OUT_OF_RANGE, FAULT_MFC_READ_UNRESOLVED,
    FAULT_UNIMPLEMENTED_INSN,
};
use crate::instruction::SpuDecodeError;
use crate::stop::SpuStopKind;
use crate::{decode, exec};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ChannelStall, ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics,
    ProblemStateError, RestartError, SignalNotifier, StallWake, StopRegisters, UnitStatus,
    YieldReason,
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
        self.state.channels.tag_status = !ctx.outstanding_dma_tags();
        // Each queued command holds a slot until it completes. A count
        // that rises from 0 here is the Qv event's edge.
        self.state.channels.cmd_queue_free =
            spu::MFC_SPU_QUEUE_DEPTH.saturating_sub(ctx.dma_queue_occupancy());
        self.state.channels.settle_tag_update();
        self.state.channels.in_mbox = ctx.inbound_mailbox().to_vec();

        // Mirror cross-unit reservation invalidation. The context view is
        // frozen for the step, so a single entry-time check suffices.
        if self.state.reservation.is_some() && !ctx.reservation_held(self.id) {
            self.state.reservation = None;
        }

        let mut remaining = budget.raw();
        effects.clear();

        loop {
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

            match exec::execute(&insn, &mut self.state, self.id) {
                SpuStepOutcome::Continue => {
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
                    effects.extend(step_effects);
                    if reason == YieldReason::ChannelStall {
                        // The access did not retire: PC stays on it, and
                        // the unit names the channel for its waker.
                        self.stall = channel_stall(&insn);
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
                    let read =
                        copy_into_local_store(&mut self.state.ls, ctx.memory(), ea, lsa, size);
                    if let Err(refusal) = read {
                        // A reservation over bytes that never arrived
                        // would let a later putllc succeed against stale
                        // local store, so a refused copy takes none.
                        let (fault, address) = match refusal {
                            CopyRefusal::Unresolved => {
                                (guest_fault(FAULT_MFC_READ_UNRESOLVED, ea as u32), ea)
                            }
                            CopyRefusal::LocalStoreEscapes => {
                                (guest_fault(FAULT_LS_OUT_OF_RANGE, lsa), u64::from(lsa))
                            }
                        };
                        self.state.reservation = None;
                        effects.clear();
                        self.status = UnitStatus::Faulted;
                        return ExecutionStepResult {
                            yield_reason: YieldReason::Fault,
                            consumed_cost: InstructionCost::new(budget.raw() - remaining),
                            local_diagnostics: LocalDiagnostics::with_pc_ea(step_pc, address),
                            fault: Some(fault),
                            syscall_args: None,
                        };
                    }
                    effects.extend(shared_read(ea, size, self.id));
                    if let Some(line_addr) = acquire_line {
                        // [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] the channel holds the status of the last completed immediate atomic command.
                        self.state.channels.atomic_status = MFC_ATOMIC_STAT_G;
                        self.state.channels.atomic_status_ready = true;
                        self.state.reservation =
                            Some(cellgov_sync::ReservedLine::containing(line_addr));
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
                        // The detail half carries the tag, so the address
                        // rides here.
                        SpuFault::MfcGetAddressWraps(_) => {
                            let c = &self.state.channels;
                            let ea = (u64::from(c.mfc_eah) << 32) | u64::from(c.mfc_eal);
                            LocalDiagnostics::with_pc_ea(step_pc, ea)
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
        let crate::state::SpuState {
            regs,
            ls,
            pc,
            lslr,
            // The replay snapshot does not carry these.
            signals: _,
            channels: _,
            reservation,
            stop,
            fpscr,
        } = &self.state;
        SpuSnapshot {
            regs: *regs,
            pc: *pc,
            lslr: *lslr,
            ls: ls.clone(),
            reservation_line: reservation.map(|l| l.addr()),
            stop: *stop,
            fpscr: *fpscr,
        }
    }

    fn stop_registers(&self) -> Option<StopRegisters> {
        self.state.stop.map(|stop| StopRegisters {
            status: stop.status_word(),
            npc: stop.npc,
        })
    }

    /// [CBEA p:95 s:8.5.3] a restart resumes at SPU_NPC; [CBEA p:94 s:8.5.2] it clears the C, I, S, H and P bits.
    fn restart(&mut self) -> Result<(), RestartError> {
        let stop = self.state.stop.take().ok_or(RestartError::NotStopped)?;
        self.state.pc = stop.npc;
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

    /// [CBEA p:95 s:8.5.3] a write updates SPU_NPC only while the SPU is stopped; its least significant bit is the interrupt-enable state, which the model does not carry.
    /// A new SPU_NPC abandons a parked channel access, which took
    /// nothing.
    fn write_npc(&mut self, npc: u32) -> Result<(), ProblemStateError> {
        if self.status == UnitStatus::Faulted {
            return Err(ProblemStateError::Refused);
        }
        let stop = self.state.stop.as_mut().ok_or(ProblemStateError::Running)?;
        stop.npc = npc & self.state.lslr & !3;
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

    /// [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox takes the oldest message out of the queue.
    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.state.channels.out_mbox.take())
    }

    fn channel_stall(&self) -> Option<ChannelStall> {
        self.stall
    }

    /// [CBEA p:60 s:7.5] a get moves main-storage bytes into local storage.
    fn land_local_store(&mut self, lsa: u32, bytes: &[u8]) -> Result<(), ProblemStateError> {
        let start = lsa as usize;
        let target = start
            .checked_add(bytes.len())
            .and_then(|end| self.state.ls.get_mut(start..end))
            .ok_or(ProblemStateError::Refused)?;
        target.copy_from_slice(bytes);
        Ok(())
    }

    fn local_memory_hash(&self) -> Option<u64> {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.state.ls);
        Some(hasher.finish())
    }
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
        spu::MFC_RD_TAG_STAT => StallWake::DmaCompletion,
        spu::SPU_WR_OUT_MBOX => StallWake::OutboundMailboxRead,
        spu::MFC_CMD => StallWake::CommandQueueSlot,
        _ => return None,
    };
    Some(ChannelStall { channel, wake })
}
