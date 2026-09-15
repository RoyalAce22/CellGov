//! Synergistic Processing Unit execution unit.
//!
//! Owns the fetch-decode-execute loop; instruction semantics live in
//! [`exec`], decoding in [`decode`]. Guest-visible writes flow through
//! `Effect` packets; reads into the 256 KB local store are serviced
//! from the frozen committed snapshot exposed by
//! [`cellgov_exec::ExecutionContext::memory`].

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod decode;
pub mod exec;
pub mod instruction;
pub mod loader;
pub mod state;

use crate::exec::{SpuFault, SpuStepOutcome};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{Budget, InstructionCost};

/// Fault code constants encoded into `FaultKind::Guest`.
const FAULT_LS_OUT_OF_RANGE: u32 = 0x0002_0000;
const FAULT_UNSUPPORTED_CHANNEL: u32 = 0x0003_0000;
const FAULT_UNSUPPORTED_MFC_CMD: u32 = 0x0004_0000;
const FAULT_DECODE_ERROR: u32 = 0x0005_0000;
/// A refused `rchcnt` keeps its own fault class. The trace then
/// distinguishes it from a refused `rdch` / `wrch` on the same channel.
const FAULT_UNSUPPORTED_CHANNEL_COUNT: u32 = 0x0006_0000;
/// A parked MFC GET whose effective address resolves to no region, or
/// whose local-store destination escapes the store. Its low bits carry
/// the transfer's tag id.
const FAULT_MFC_GET_UNRESOLVED: u32 = 0x0007_0000;
/// An MFC command whose staged tag id is outside 0..31. The low bits
/// carry the value the guest wrote, masked to 16 bits.
const FAULT_MFC_TAG_ID_OUT_OF_RANGE: u32 = 0x0008_0000;
/// A synchronous MFC read -- `getllar` -- whose effective address
/// resolves to no region, or whose local-store destination escapes the
/// store. Either arm carries the low 16 bits of the effective address,
/// masked so the detail cannot reach the class field.
///
/// Distinct from [`FAULT_MFC_GET_UNRESOLVED`] so a trace separates a
/// line the atomic path never read from a parked transfer that never
/// landed.
const FAULT_MFC_READ_UNRESOLVED: u32 = 0x0009_0000;

/// Records the bytes a transfer copied from main memory into local store.
///
/// Dependency analysis pairs the range against another unit's write to
/// the same bytes. The result is `None` for:
///
/// - a zero-byte transfer, whose empty range pairs with no write;
/// - an `ea + size` that carries out of the 64-bit space.
fn shared_read(ea: u64, size: u32, source: UnitId) -> Option<Effect> {
    // [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] Zero is a valid MFC transfer size.
    if size == 0 {
        return None;
    }
    cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ea), u64::from(size))
        .map(|range| Effect::SharedReadIntent { range, source })
}

/// SPU execution unit snapshot for replay.
#[derive(Debug, Clone)]
pub struct SpuSnapshot {
    /// Register file.
    pub regs: [[u8; 16]; 128],
    /// Program counter.
    pub pc: u32,
    /// Local store contents.
    pub ls: Vec<u8>,
    /// Canonical line address of the atomic reservation; `None` when
    /// no reservation is held.
    pub reservation_line: Option<u64>,
}

/// A Synergistic Processing Unit execution unit.
#[derive(Clone)]
pub struct SpuExecutionUnit {
    id: UnitId,
    state: state::SpuState,
    status: UnitStatus,
}

impl SpuExecutionUnit {
    /// Construct a runnable SPU with zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::SpuState::new(),
            status: UnitStatus::Runnable,
        }
    }

    /// Mutable access to architectural state.
    pub fn state_mut(&mut self) -> &mut state::SpuState {
        &mut self.state
    }

    /// Read access to architectural state.
    pub fn state(&self) -> &state::SpuState {
        &self.state
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
        // Re-entry after a mailbox park: a yielded rdch leaves PC on the
        // instruction; resume by writing the message and stepping past it.
        if let Some(&msg) = ctx.received_messages().first() {
            let rt = self.state.channels.pending_mbox_rt.take().unwrap_or(2);
            self.state.set_reg_word_splat(rt, msg);
            self.state.pc += 4;
        }

        // This step clears the effect vector below, so the parked
        // transfer's read enters it after the clear.
        let mut parked_get_read = None;
        if let Some((ea, lsa, size, tag_id)) = self.state.channels.pending_get.take() {
            // The guest writes `ea` through MFC_EAH and MFC_EAL, so it
            // can name anything, an address no region backs included.
            // Resolving through the memory's own read reaches whichever
            // region backs it; a slice of `as_bytes()` reaches only the
            // region at the base.
            // [CBEA p:111 s:9 SPU Channel Map] MFC_EAL is a write channel carrying the low-order SPU effective-address command parameter.
            // A transfer of no bytes reads no main storage and writes no
            // local store, so neither address has to resolve for it.
            // [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] Zero is a valid MFC transfer size.
            let moved = size == 0
                || ByteRange::new(GuestAddr::new(ea), u64::from(size))
                    .and_then(|src| ctx.memory().read(src))
                    .and_then(|bytes| {
                        let dst_start = lsa as usize;
                        let dst_end = dst_start.checked_add(size as usize)?;
                        let slot = self.state.ls.get_mut(dst_start..dst_end)?;
                        slot.copy_from_slice(bytes);
                        Some(())
                    })
                    .is_some();
            if !moved {
                // The tag bit is the guest's only signal that the
                // transfer finished. Publishing it here would report a
                // completion over local store the transfer never wrote,
                // so the refusal is named instead.
                effects.clear();
                self.status = UnitStatus::Faulted;
                return ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_cost: InstructionCost::new(0),
                    local_diagnostics: LocalDiagnostics::with_pc(self.state.pc as u64),
                    fault: Some(FaultKind::Guest(
                        FAULT_MFC_GET_UNRESOLVED | u32::from(tag_id),
                    )),
                    syscall_args: None,
                };
            }
            parked_get_read = shared_read(ea, size, self.id);
            self.state.channels.tag_status |= 1u32 << tag_id;
        }
        self.state.channels.tag_status |= ctx.completed_dma_tags();

        // Mirror cross-unit reservation invalidation. The context view is
        // frozen for the step, so a single entry-time check suffices.
        if self.state.reservation.is_some() && !ctx.reservation_held(self.id) {
            self.state.reservation = None;
        }

        let mut remaining = budget.raw();
        effects.clear();
        effects.extend(parked_get_read);

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
                        fault: Some(FaultKind::Guest(FAULT_LS_OUT_OF_RANGE | self.state.pc)),
                        syscall_args: None,
                    };
                }
            };

            let insn = match decode::decode(raw) {
                Ok(i) => i,
                Err(_) => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_DECODE_ERROR)),
                        syscall_args: None,
                    };
                }
            };

            match exec::execute(&insn, &mut self.state, self.id) {
                SpuStepOutcome::Continue => {
                    self.state.pc += 4;
                }
                SpuStepOutcome::Branch => {}
                SpuStepOutcome::Yield {
                    effects: step_effects,
                    reason,
                } => {
                    effects.extend(step_effects);
                    if reason == YieldReason::Finished {
                        self.status = UnitStatus::Finished;
                    } else if reason != YieldReason::MailboxAccess {
                        // Mailbox path keeps PC on the rdch for retry; the
                        // re-entry block at the top of run_until_yield
                        // advances PC once a message lands.
                        self.state.pc += 4;
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
                    // Resolved through the memory's own read, so the
                    // line reaches whichever region backs it. The guest
                    // writes `ea` through MFC_EAH and MFC_EAL, so it can
                    // also name an address no region backs.
                    let read = ByteRange::new(GuestAddr::new(ea), u64::from(size))
                        .and_then(|src| ctx.memory().read(src))
                        .and_then(|bytes| {
                            let dst_start = lsa as usize;
                            let dst_end = dst_start.checked_add(size as usize)?;
                            let slot = self.state.ls.get_mut(dst_start..dst_end)?;
                            slot.copy_from_slice(bytes);
                            Some(())
                        });
                    if read.is_none() {
                        // The reservation is the guest's evidence that
                        // it holds the line. Acquiring one over bytes
                        // that never arrived would let a later putllc
                        // succeed against a comparison made on stale
                        // local store, so the refusal is named and no
                        // reservation is taken.
                        self.state.reservation = None;
                        effects.clear();
                        self.status = UnitStatus::Faulted;
                        return ExecutionStepResult {
                            yield_reason: YieldReason::Fault,
                            consumed_cost: InstructionCost::new(budget.raw() - remaining),
                            local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                            fault: Some(FaultKind::Guest(
                                FAULT_MFC_READ_UNRESOLVED | (ea as u32 & 0xFFFF),
                            )),
                            syscall_args: None,
                        };
                    }
                    effects.extend(shared_read(ea, size, self.id));
                    // MFC_GETLLAR also installs the unit's reservation entry.
                    if let Some(line_addr) = acquire_line {
                        effects.push(Effect::ReservationAcquire {
                            line_addr,
                            source: self.id,
                        });
                    }
                    self.state.pc += 4;
                }
                SpuStepOutcome::Fault(f) => {
                    self.status = UnitStatus::Faulted;
                    let code = match f {
                        SpuFault::LsOutOfRange(a) => FAULT_LS_OUT_OF_RANGE | a,
                        SpuFault::UnsupportedChannel { channel, .. } => {
                            FAULT_UNSUPPORTED_CHANNEL | channel as u32
                        }
                        SpuFault::UnsupportedMfcCommand(c) => FAULT_UNSUPPORTED_MFC_CMD | c,
                        SpuFault::UnsupportedChannelCount(channel) => {
                            FAULT_UNSUPPORTED_CHANNEL_COUNT | channel as u32
                        }
                        // The detail is masked: the staged tag is
                        // whatever the guest wrote to the channel, so it
                        // would otherwise smear into the class bits.
                        SpuFault::TagIdOutOfRange(tag) => {
                            FAULT_MFC_TAG_ID_OUT_OF_RANGE | (tag & 0xFFFF)
                        }
                    };
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(code)),
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
        SpuSnapshot {
            regs: self.state.regs,
            pc: self.state.pc,
            ls: self.state.ls.clone(),
            reservation_line: self.state.reservation.map(|l| l.addr()),
        }
    }
}

#[cfg(test)]
#[path = "tests/read_intent_tests.rs"]
mod read_intent_tests;

#[cfg(test)]
#[path = "tests/parked_get_tests.rs"]
mod parked_get_tests;

#[cfg(test)]
#[path = "tests/tag_id_tests.rs"]
mod tag_id_tests;

#[cfg(test)]
#[path = "tests/getllar_tests.rs"]
mod getllar_tests;

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
