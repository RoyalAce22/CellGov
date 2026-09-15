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
use cellgov_ps3_abi::hw::spu::MFC_ATOMIC_STAT_G;
use cellgov_time::{Budget, InstructionCost};

// Fault code constants encoded into `FaultKind::Guest`.

/// A fetch or an access outside local store.
///
/// The detail is one of:
///
/// - the program counter, on the fetch path;
/// - the raw address operand, on the load/store path;
/// - the staged `MFC_LSA`, where an MFC put, get, getllar or putllc
///   names a range local store cannot hold.
///
/// Local store spans 18 bits, so none of them fits the detail half and
/// the masked value gives the address modulo 64 KB. [`LocalDiagnostics`]
/// carries the whole value beside the code: the fetch path's program
/// counter as `pc`, the other two as `faulting_ea`.
// [CBE-Handbook p:64 s:3.1.1 Local Store] Local store holds 256 KB, so an address inside it needs 18 bits.
const FAULT_LS_OUT_OF_RANGE: u32 = 0x0002_0000;
const FAULT_UNSUPPORTED_CHANNEL: u32 = 0x0003_0000;
/// An MFC command the model has no arm for.
///
/// The detail is the command word the guest wrote. The opcode sits in
/// its low byte, so the masked detail still names the refused command;
/// the two class ids above the mask say nothing about which it was.
// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] The word written to this channel carries the transfer and replacement class ids in its high half and the MFC command opcode in its low byte.
const FAULT_UNSUPPORTED_MFC_CMD: u32 = 0x0004_0000;
const FAULT_DECODE_ERROR: u32 = 0x0005_0000;
/// A refused `rchcnt` keeps its own fault class. The trace then
/// distinguishes it from a refused `rdch` / `wrch` on the same channel.
const FAULT_UNSUPPORTED_CHANNEL_COUNT: u32 = 0x0006_0000;
/// A parked MFC GET whose effective address resolves to no region. Its
/// low bits carry the transfer's tag id; [`LocalDiagnostics::faulting_ea`]
/// carries the effective address whole. A destination that escapes
/// local store is [`FAULT_LS_OUT_OF_RANGE`], as it is for a put.
const FAULT_MFC_GET_UNRESOLVED: u32 = 0x0007_0000;
/// An MFC command whose staged tag id is outside 0..31. The low bits
/// carry the value the guest wrote, masked to 16 bits.
const FAULT_MFC_TAG_ID_OUT_OF_RANGE: u32 = 0x0008_0000;
/// A synchronous MFC read -- `getllar` -- whose effective address
/// resolves to no region. The detail carries the low 16 bits of the
/// effective address, masked so it cannot reach the class field;
/// [`LocalDiagnostics::faulting_ea`] carries the whole address. A
/// destination that escapes local store is [`FAULT_LS_OUT_OF_RANGE`].
///
/// Distinct from [`FAULT_MFC_GET_UNRESOLVED`] so a trace separates a
/// line the atomic path never read from a parked transfer that never
/// landed.
const FAULT_MFC_READ_UNRESOLVED: u32 = 0x0009_0000;

/// The half of a fault code that carries the detail.
///
/// A class occupies the half above it. `cellgov_boot`'s fault report
/// splits a guest code at the same halfword boundary, so a detail that
/// reached a class bit would print as a different fault.
const FAULT_DETAIL_MASK: u32 = 0xFFFF;

/// Every class this crate raises, so the layout checks and the layout
/// tests cover one set.
const EVERY_FAULT_CLASS: [u32; 8] = [
    FAULT_LS_OUT_OF_RANGE,
    FAULT_UNSUPPORTED_CHANNEL,
    FAULT_UNSUPPORTED_MFC_CMD,
    FAULT_DECODE_ERROR,
    FAULT_UNSUPPORTED_CHANNEL_COUNT,
    FAULT_MFC_GET_UNRESOLVED,
    FAULT_MFC_TAG_ID_OUT_OF_RANGE,
    FAULT_MFC_READ_UNRESOLVED,
];

// `guest_fault`'s debug assertion compiles out under `--release`, which
// is the profile a trace a reader decodes comes from, so the classes
// that exist are held against the layout here instead. Distinctness
// belongs with them: masking a detail away is no use if two classes
// share a code.
const _: () = {
    // The premise for masking at all: local store spans 18 bits, so an
    // address inside it does not fit the detail half.
    assert!(
        state::SPU_LS_SIZE as u32 > FAULT_DETAIL_MASK,
        "a local store inside the detail field would leave nothing to mask",
    );
    let mut i = 0;
    while i < EVERY_FAULT_CLASS.len() {
        assert!(
            EVERY_FAULT_CLASS[i] & FAULT_DETAIL_MASK == 0,
            "a fault class reaches into the detail field",
        );
        let mut j = i + 1;
        while j < EVERY_FAULT_CLASS.len() {
            assert!(
                EVERY_FAULT_CLASS[i] != EVERY_FAULT_CLASS[j],
                "two fault classes share a code",
            );
            j += 1;
        }
        i += 1;
    }
};

/// The class and detail each [`SpuFault`] reports.
fn guest_fault_for(fault: SpuFault) -> FaultKind {
    match fault {
        SpuFault::LsOutOfRange(a) => guest_fault(FAULT_LS_OUT_OF_RANGE, a),
        SpuFault::UnsupportedChannel { channel, .. } => {
            guest_fault(FAULT_UNSUPPORTED_CHANNEL, channel as u32)
        }
        SpuFault::UnsupportedMfcCommand(c) => guest_fault(FAULT_UNSUPPORTED_MFC_CMD, c),
        SpuFault::UnsupportedChannelCount(channel) => {
            guest_fault(FAULT_UNSUPPORTED_CHANNEL_COUNT, channel as u32)
        }
        SpuFault::TagIdOutOfRange(tag) => guest_fault(FAULT_MFC_TAG_ID_OUT_OF_RANGE, tag),
    }
}

/// One guest fault: `class` in the high half of the code, `detail`
/// masked into the low half.
///
/// Each detail these paths carry is a value the guest picked or
/// influenced -- a program counter, a channel number, an MFC command
/// word, a tag id -- so an unmasked one sets a class bit and the code
/// decodes as some other fault. The assertion covers a class added
/// after [`EVERY_FAULT_CLASS`].
fn guest_fault(class: u32, detail: u32) -> FaultKind {
    debug_assert!(
        class & FAULT_DETAIL_MASK == 0,
        "fault class 0x{class:08x} reaches into the detail field",
    );
    FaultKind::Guest(class | (detail & FAULT_DETAIL_MASK))
}

/// Which end of a main-memory-to-local-store copy refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyRefusal {
    /// No region backs the source range.
    Unresolved,
    /// The destination range escapes local store.
    LocalStoreEscapes,
}

/// Copy `size` bytes of committed memory at `ea` into local store at
/// `lsa`, or name the end that refused.
///
/// The source is tested first, so an escaping destination refuses only
/// where the source resolves. Either refusal leaves local store
/// untouched.
fn copy_into_local_store(
    ls: &mut [u8],
    memory: &cellgov_mem::GuestMemory,
    ea: u64,
    lsa: u32,
    size: u32,
) -> Result<(), CopyRefusal> {
    let bytes = ByteRange::new(GuestAddr::new(ea), u64::from(size))
        .and_then(|src| memory.read(src))
        .ok_or(CopyRefusal::Unresolved)?;
    let dst_start = lsa as usize;
    let slot = dst_start
        .checked_add(size as usize)
        .and_then(|end| ls.get_mut(dst_start..end))
        .ok_or(CopyRefusal::LocalStoreEscapes)?;
    slot.copy_from_slice(bytes);
    Ok(())
}

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
            let moved = if size == 0 {
                Ok(())
            } else {
                copy_into_local_store(&mut self.state.ls, ctx.memory(), ea, lsa, size)
            };
            if let Err(refusal) = moved {
                // The tag bit is the guest's only signal that the
                // transfer finished. Publishing it here would report a
                // completion over local store the transfer never wrote,
                // so the refusal is named instead, by the end that
                // refused: the address that end names rides whole beside
                // the code.
                let (fault, address) = match refusal {
                    CopyRefusal::Unresolved => {
                        (guest_fault(FAULT_MFC_GET_UNRESOLVED, u32::from(tag_id)), ea)
                    }
                    CopyRefusal::LocalStoreEscapes => {
                        (guest_fault(FAULT_LS_OUT_OF_RANGE, lsa), u64::from(lsa))
                    }
                };
                effects.clear();
                self.status = UnitStatus::Faulted;
                return ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_cost: InstructionCost::new(0),
                    local_diagnostics: LocalDiagnostics::with_pc_ea(self.state.pc as u64, address),
                    fault: Some(fault),
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
                        fault: Some(guest_fault(FAULT_LS_OUT_OF_RANGE, self.state.pc)),
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
                        fault: Some(guest_fault(FAULT_DECODE_ERROR, 0)),
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
                    let read =
                        copy_into_local_store(&mut self.state.ls, ctx.memory(), ea, lsa, size);
                    if let Err(refusal) = read {
                        // The reservation is the guest's evidence that
                        // it holds the line. Acquiring one over bytes
                        // that never arrived would let a later putllc
                        // succeed against a comparison made on stale
                        // local store, so the refusal is named and no
                        // reservation is taken. The end that refused
                        // names the fault, and its address rides whole
                        // beside the code.
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
                    // MFC_GETLLAR also installs the unit's reservation
                    // entry and its atomic status. Both land here, after
                    // the line arrives, so a refused read reports no
                    // status and holds no reservation.
                    if let Some(line_addr) = acquire_line {
                        // [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] the channel holds the status of the last completed immediate atomic command.
                        self.state.channels.atomic_status = MFC_ATOMIC_STAT_G;
                        self.state.reservation =
                            Some(cellgov_sync::ReservedLine::containing(line_addr));
                        effects.push(Effect::ReservationAcquire {
                            line_addr,
                            source: self.id,
                        });
                    }
                    self.state.pc += 4;
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

    fn local_memory_hash(&self) -> Option<u64> {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.state.ls);
        Some(hasher.finish())
    }
}

#[cfg(test)]
#[path = "tests/read_intent_tests.rs"]
mod read_intent_tests;

#[cfg(test)]
#[path = "tests/parked_get_tests.rs"]
mod parked_get_tests;

#[cfg(test)]
#[path = "tests/fault_code_tests.rs"]
mod fault_code_tests;

#[cfg(test)]
#[path = "tests/tag_id_tests.rs"]
mod tag_id_tests;

#[cfg(test)]
#[path = "tests/getllar_tests.rs"]
mod getllar_tests;

#[cfg(test)]
#[path = "tests/atomic_line_tests.rs"]
mod atomic_line_tests;

#[cfg(test)]
#[path = "tests/fault_diag_tests.rs"]
mod fault_diag_tests;

#[cfg(test)]
#[path = "tests/local_memory_hash_tests.rs"]
mod local_memory_hash_tests;

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
