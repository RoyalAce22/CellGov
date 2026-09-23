//! Runs internal PPU paths for differential optimization-consistency checks. Agreement does not establish hardware accuracy.
//!
//! [Jiang2022 p:12 s:6.1] Agreement with a trusted emulator proves nothing about hardware unless something else shows that emulator follows the specification.

use std::collections::BTreeSet;
use std::{cell::RefCell, rc::Rc};

use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionUnit, FaultRegisterDump, LocalDiagnostics, YieldReason,
};
use cellgov_mem::RegionView;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region};
use cellgov_ppu::decode::decode;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::observation::{
    finish_observation, PpuArchitecturalState, PpuObservation, PpuObservationCheck,
    PpuObservationComponent, PpuObservationError, PpuObservationInput, PpuObservedOutcome,
};
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_sync::{ReservationTable, ReservedLine};
use cellgov_time::Budget;

const UNIT: UnitId = UnitId::new(0);
const DATA_BASE: u64 = 0x1000_0000;

/// One internal PPU execution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuExecutionPath {
    /// Executes one instruction per batch with live decode.
    Plain,
    /// Uses store forwarding in a multi-instruction batch with live decode.
    Forwarded,
    /// Uses a quickened shadow in a multi-instruction batch.
    Quickened,
    /// Uses quickening and fused pairs in a multi-instruction batch.
    Fused,
}

/// Records why an internal PPU path stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuPathStop {
    /// The yield reason from the final runtime step.
    pub reason: YieldReason,
    /// The guest fault from the final runtime step, if any.
    pub fault: Option<cellgov_effects::FaultKind>,
    /// The program counter from the final runtime step.
    pub pc: Option<u64>,
    /// Full fault-site or syscall diagnostics.
    pub diagnostics: LocalDiagnostics,
    /// Raw syscall number and arguments.
    pub syscall_args: Option<[u64; 9]>,
}

/// Records the final result for one internal PPU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuPathRun {
    /// The execution path that produced this run.
    pub path: PpuExecutionPath,
    /// The final PPU state and effects.
    pub observation: PpuObservation,
    /// The terminal runtime attribution.
    pub stop: PpuPathStop,
    /// The number of instruction slots that retired.
    pub retired: u64,
    /// Program counters dispatched in execution order.
    pub executed_pcs: Vec<u64>,
    /// Committed-memory reads in data-region order, excluding fully forwarded loads.
    pub committed_data_reads: Vec<ByteRange>,
}

#[derive(Default)]
struct DispatchTrace(RefCell<Vec<u64>>);

impl cellgov_ppu::PpuTap for DispatchTrace {
    fn dispatch(
        &self,
        _unit: UnitId,
        _insn: &cellgov_ppu::instruction::PpuInstruction,
        state: &PpuState,
    ) {
        self.0.borrow_mut().push(state.pc);
    }
}

/// First typed disagreement between two internal paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuPathDivergence {
    /// The earlier path in the compared pair.
    pub left: PpuExecutionPath,
    /// The later path in the compared pair.
    pub right: PpuExecutionPath,
    /// The observation components that differ.
    pub observation: BTreeSet<PpuObservationComponent>,
    /// Whether the terminal runtime attribution differs.
    pub stop_differs: bool,
    /// Whether the retired instruction count differs.
    pub retired_differs: bool,
    /// Whether the committed data-read footprints differ.
    pub data_reads_differ: bool,
}

/// Failure to construct or observe an internal path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpuPathError {
    /// Guest-memory construction or commit returned an error.
    #[error("PPU path memory failed: {0}")]
    Memory(#[from] MemError),
    /// The shared observation contract refused the batch.
    #[error("PPU path observation failed: {0}")]
    Observation(#[from] PpuObservationError),
    /// A path emitted effects that the observation contract refused to commit.
    #[error("PPU path {path:?} commit refused: {error}")]
    CommitRefusal {
        /// Path that emitted the refused batch.
        path: PpuExecutionPath,
        /// Typed commit refusal.
        #[source]
        error: PpuObservationError,
        /// Effects staged before the commit boundary.
        staged_effects: Vec<cellgov_effects::Effect>,
        /// Effects accepted by the commit boundary.
        committed_effects: Vec<cellgov_effects::Effect>,
    },
    /// A write exceeds the forwarding model's entry width.
    #[error("PPU path forwarding write has unsupported width {length}")]
    ForwardingWidth {
        /// Width in bytes.
        length: u64,
    },
    /// The forwarding model cannot hold another write.
    #[error("PPU path forwarding buffer is full at address 0x{addr:016x}")]
    ForwardingCapacity {
        /// First guest byte of the refused write.
        addr: u64,
    },
    /// The forwarding model refused a write for a reason other than capacity.
    #[error("PPU path forwarding refused a write: {source}")]
    ForwardingRefused {
        /// The store buffer's own refusal.
        #[source]
        source: cellgov_ppu::store_buffer::StoreRefusal,
    },
    /// The instruction sequence contains no instructions.
    #[error("PPU path sequence must contain at least one instruction")]
    EmptySequence,
    /// The data region contains no bytes.
    #[error("PPU path data region must contain at least one byte")]
    EmptyData,
    /// The range exceeds the guest-address space.
    #[error("PPU path range at 0x{base:016x} of {size} bytes overflows")]
    RangeOverflow {
        /// The first guest address in the rejected range.
        base: u64,
        /// The rejected range length.
        size: u64,
    },
    /// The instruction count exceeds the guest budget range.
    #[error("PPU path sequence of {words} instructions exceeds the guest budget range")]
    BudgetOverflow {
        /// The instruction count that exceeds the guest budget range.
        words: usize,
    },
    /// A requested code rewrite does not name a sequence word.
    #[error("PPU path code mutation word {word_index} is outside {words} words")]
    CodeMutationOutOfRange {
        /// Rejected word index.
        word_index: usize,
        /// Number of generated words.
        words: usize,
    },
}

/// Compares internal paths with the same instruction sequence, PPU state, and data.
// [McKeeman1998 p:101 s:Differential Testing] One generated test goes to several comparable systems, and a differing result is a candidate bug.
pub fn run_all_paths(
    words: &[u32],
    initial: &PpuState,
    data: &[u8],
) -> Result<Vec<PpuPathRun>, PpuPathError> {
    run_paths(words, initial, data, None)
}

/// Runs one internal path through the same observation contract as a full comparison.
pub fn run_one_path(
    path: PpuExecutionPath,
    words: &[u32],
    initial: &PpuState,
    data: &[u8],
) -> Result<PpuPathRun, PpuPathError> {
    if words.is_empty() {
        return Err(PpuPathError::EmptySequence);
    }
    if data.is_empty() {
        return Err(PpuPathError::EmptyData);
    }
    let code = words
        .iter()
        .copied()
        .chain(std::iter::repeat_n(24 << 26, words.len()))
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    run_path(path, &code, &code, words.len(), initial, data, None)
}

/// Compares internal paths after rewriting one word and invalidating any cached shadow slots.
pub fn run_all_paths_after_code_mutation(
    words: &[u32],
    initial: &PpuState,
    data: &[u8],
    word_index: usize,
    replacement: u32,
) -> Result<Vec<PpuPathRun>, PpuPathError> {
    if word_index >= words.len() {
        return Err(PpuPathError::CodeMutationOutOfRange {
            word_index,
            words: words.len(),
        });
    }
    run_paths(words, initial, data, Some((word_index, replacement)))
}

fn run_paths(
    words: &[u32],
    initial: &PpuState,
    data: &[u8],
    mutation: Option<(usize, u32)>,
) -> Result<Vec<PpuPathRun>, PpuPathError> {
    if words.is_empty() {
        return Err(PpuPathError::EmptySequence);
    }
    if data.is_empty() {
        return Err(PpuPathError::EmptyData);
    }
    let shadow_code = words
        .iter()
        .copied()
        .chain(std::iter::repeat_n(24 << 26, words.len()))
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    let mut code = shadow_code.clone();
    if let Some((word_index, replacement)) = mutation {
        let start = word_index * 4;
        code[start..start + 4].copy_from_slice(&replacement.to_be_bytes());
    }
    [
        PpuExecutionPath::Plain,
        PpuExecutionPath::Forwarded,
        PpuExecutionPath::Quickened,
        PpuExecutionPath::Fused,
    ]
    .into_iter()
    .map(|path| {
        run_path(
            path,
            &code,
            &shadow_code,
            words.len(),
            initial,
            data,
            mutation.map(|(word_index, _)| (word_index * 4) as u64),
        )
    })
    .collect()
}

/// Scans runs in caller order and returns their first disagreement.
// [Wang2024 p:340:3 s:2.1] Optimization settings of one implementation count as separate testing backends. A differing result on a well defined deterministic program means at least one backend is wrong.
pub fn first_path_divergence(runs: &[PpuPathRun]) -> Option<PpuPathDivergence> {
    for (index, left) in runs.iter().enumerate() {
        for right in runs.iter().skip(index + 1) {
            let comparison = left
                .observation
                .compare(&right.observation, PpuObservationCheck::DeterministicReplay);
            let stop_differs = left.stop != right.stop;
            let retired_differs = left.retired != right.retired;
            let data_reads_differ = left.committed_data_reads != right.committed_data_reads;
            if !comparison.complete_differences.is_empty()
                || stop_differs
                || retired_differs
                || data_reads_differ
            {
                return Some(PpuPathDivergence {
                    left: left.path,
                    right: right.path,
                    observation: comparison.complete_differences,
                    stop_differs,
                    retired_differs,
                    data_reads_differ,
                });
            }
        }
    }
    None
}

fn run_path(
    path: PpuExecutionPath,
    code: &[u8],
    shadow_code: &[u8],
    words: usize,
    initial: &PpuState,
    initial_data: &[u8],
    invalidated_pc: Option<u64>,
) -> Result<PpuPathRun, PpuPathError> {
    if path == PpuExecutionPath::Plain {
        return run_plain(code, words, initial, initial_data);
    }
    let budget =
        Budget::new(u64::try_from(words).map_err(|_| PpuPathError::BudgetOverflow { words })?);
    let mut unit = PpuExecutionUnit::new(UNIT);
    *unit.state_mut() = initial.clone();
    let trace = Rc::new(DispatchTrace::default());
    unit.set_tap(trace.clone());
    match path {
        PpuExecutionPath::Quickened => {
            let mut shadow = PredecodedShadow::build_quickened(0, shadow_code);
            if let Some(pc) = invalidated_pc {
                shadow.invalidate_range(pc, 4);
            }
            unit.set_instruction_shadow(shadow);
        }
        PpuExecutionPath::Fused => {
            let mut shadow = PredecodedShadow::build(0, shadow_code);
            if let Some(pc) = invalidated_pc {
                shadow.invalidate_range(pc, 4);
            }
            unit.set_instruction_shadow(shadow);
        }
        PpuExecutionPath::Plain | PpuExecutionPath::Forwarded => {}
    }

    let mut data = initial_data.to_vec();
    let mut reservations = initial_reservations(initial);
    let mut effects_all = Vec::new();
    let mut committed_data_reads = Vec::new();
    let mut retired = 0u64;
    let mut final_result = None;
    let batches = 1;
    for _ in 0..batches {
        let memory = make_memory(code, &data)?;
        let entry = unit.state().clone();
        let mut effects = Vec::new();
        let ctx = ExecutionContext::new(&memory).with_reservations(&reservations);
        let result = unit.run_until_yield(budget, &ctx, &mut effects);
        retired = retired.saturating_add(result.consumed_cost.raw());
        let observed = finish_observation(PpuObservationInput {
            initial_state: &entry,
            final_state: unit.state(),
            memory_base: DATA_BASE,
            initial_memory: &data,
            outcome: PpuObservedOutcome::RuntimeStep(Box::new(result.clone())),
            effects,
            stores: StoreBuffer::new(),
            unit: UNIT,
        })?;
        refuse_commit(path, &observed)?;
        committed_data_reads.extend(observed.committed_effects.iter().filter_map(|effect| {
            match effect {
                cellgov_effects::Effect::SharedReadIntent { range, .. }
                    if range.start().raw() >= DATA_BASE =>
                {
                    Some(*range)
                }
                _ => None,
            }
        }));
        data = observed.memory;
        reservations = reservation_table(&observed.reservations);
        effects_all.extend(
            observed
                .committed_effects
                .into_iter()
                .filter(|effect| !is_read_intent(effect)),
        );
        let reason = result.yield_reason;
        final_result = Some(result);
        if reason != YieldReason::BudgetExhausted {
            break;
        }
    }
    let result = final_result.ok_or(PpuPathError::EmptySequence)?;
    let stop = PpuPathStop {
        reason: result.yield_reason,
        fault: result.fault,
        pc: result.local_diagnostics.pc,
        diagnostics: result.local_diagnostics,
        syscall_args: result.syscall_args,
    };
    let executed_pcs = trace.0.borrow().clone();
    let mut observation = PpuObservation {
        state: PpuArchitecturalState::capture(unit.state()),
        memory: data,
        outcome: PpuObservedOutcome::NoInstruction,
        staged_effects: effects_all.clone(),
        committed_effects: effects_all,
        commit_error: None,
        reservations: reservations.iter().collect(),
        store_buffer: Vec::new(),
        fault_discarded: stop.reason == YieldReason::Fault,
    };
    crate::seeded::ppu_observed(&mut observation);
    Ok(PpuPathRun {
        path,
        observation,
        stop,
        retired,
        executed_pcs,
        committed_data_reads,
    })
}

fn run_plain(
    code: &[u8],
    words: usize,
    initial: &PpuState,
    initial_data: &[u8],
) -> Result<PpuPathRun, PpuPathError> {
    let mut state = initial.clone();
    let mut data = initial_data.to_vec();
    let mut reservations = initial_reservations(initial);
    let mut effects_all = Vec::new();
    let mut committed_data_reads = Vec::new();
    let mut forwarding = StoreBuffer::new();
    let mut executed_pcs = Vec::new();
    let mut retired = 0u64;
    let mut stop = PpuPathStop {
        reason: YieldReason::BudgetExhausted,
        fault: None,
        pc: Some(0),
        diagnostics: LocalDiagnostics::with_pc(0),
        syscall_args: None,
    };
    for _ in 0..words {
        let step_pc = state.pc;
        let Some(index) = step_pc
            .checked_div(4)
            .and_then(|value| usize::try_from(value).ok())
        else {
            break;
        };
        let Some(start) = index.checked_mul(4) else {
            break;
        };
        let Some(end) = start.checked_add(4) else {
            break;
        };
        let Some(bytes) = code.get(start..end) else {
            break;
        };
        let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let Ok(instruction) = decode(raw) else {
            let diagnostics = fault_diagnostics(&state, step_pc, None);
            state = initial.clone();
            data.copy_from_slice(initial_data);
            reservations = initial_reservations(initial);
            effects_all.clear();
            committed_data_reads.clear();
            forwarding.clear();
            retired = 0;
            stop = PpuPathStop {
                reason: YieldReason::Fault,
                fault: Some(FaultKind::Guest(cellgov_ppu::FAULT_DECODE_ERROR)),
                pc: Some(step_pc),
                diagnostics,
                syscall_args: None,
            };
            break;
        };
        let entry = state.clone();
        executed_pcs.push(step_pc);
        let mut effects = Vec::new();
        let mut stores = StoreBuffer::new();
        let views = [
            RegionView::plain(0, code),
            RegionView::plain(DATA_BASE, &data),
        ];
        let verdict = execute(
            &instruction,
            &mut state,
            UNIT,
            &views,
            &mut effects,
            &mut stores,
        );
        stores.flush(&mut effects, UNIT);
        match &verdict {
            ExecuteVerdict::Continue => state.pc = state.pc.wrapping_add(4),
            ExecuteVerdict::Branch => {}
            ExecuteVerdict::Fault(fault) => {
                let ea = match fault {
                    cellgov_ppu::exec::PpuFault::PcOutOfRange(addr)
                    | cellgov_ppu::exec::PpuFault::InvalidAddress(addr)
                    | cellgov_ppu::exec::PpuFault::AlignmentInterrupt(addr) => Some(*addr),
                    cellgov_ppu::exec::PpuFault::UnsupportedSyscall(_)
                    | cellgov_ppu::exec::PpuFault::UnimplementedInstruction(_)
                    | cellgov_ppu::exec::PpuFault::ProgramTrap(_) => None,
                };
                let diagnostics = fault_diagnostics(&state, step_pc, ea);
                state = initial.clone();
                data.copy_from_slice(initial_data);
                reservations = initial_reservations(initial);
                effects_all.clear();
                committed_data_reads.clear();
                forwarding.clear();
                retired = 0;
                stop = PpuPathStop {
                    reason: YieldReason::Fault,
                    fault: Some(FaultKind::Guest(fault.guest_code())),
                    pc: Some(step_pc),
                    diagnostics,
                    syscall_args: None,
                };
                break;
            }
            ExecuteVerdict::MemFault(error) => {
                let ea = match error {
                    MemError::Unmapped(context) => Some(context.addr),
                    MemError::ReservedWrite { addr, .. }
                    | MemError::ReservedStrictRead { addr, .. } => Some(*addr),
                    MemError::LengthMismatch
                    | MemError::OverlappingRegions
                    | MemError::RegionOverflow { .. } => None,
                };
                let diagnostics = fault_diagnostics(&state, step_pc, ea);
                state = initial.clone();
                data.copy_from_slice(initial_data);
                reservations = initial_reservations(initial);
                effects_all.clear();
                committed_data_reads.clear();
                forwarding.clear();
                retired = 0;
                stop = PpuPathStop {
                    reason: YieldReason::Fault,
                    fault: Some(FaultKind::Guest(cellgov_ppu::FAULT_INVALID_ADDRESS)),
                    pc: Some(step_pc),
                    diagnostics,
                    syscall_args: None,
                };
                break;
            }
            ExecuteVerdict::Syscall { lev } => {
                stop = PpuPathStop {
                    reason: YieldReason::Syscall,
                    fault: None,
                    pc: Some(step_pc),
                    diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(
                        step_pc,
                        state.lr(),
                        *lev,
                    ),
                    syscall_args: Some(cellgov_ppu::state::ppu_syscall_args(&state)),
                };
                break;
            }
            ExecuteVerdict::BufferFull => {
                stop.pc = Some(step_pc);
                stop.diagnostics = LocalDiagnostics::with_pc(step_pc);
                break;
            }
        }
        retired = retired.saturating_add(1);
        let observed = finish_observation(PpuObservationInput {
            initial_state: &entry,
            final_state: &state,
            memory_base: DATA_BASE,
            initial_memory: &data,
            outcome: PpuObservedOutcome::Execution(verdict),
            effects,
            stores: StoreBuffer::new(),
            unit: UNIT,
        })?;
        refuse_commit(PpuExecutionPath::Plain, &observed)?;
        committed_data_reads.extend(observed.committed_effects.iter().filter_map(|effect| {
            match effect {
                cellgov_effects::Effect::SharedReadIntent { range, .. }
                    if u8::try_from(range.length()).ok().is_none_or(|len| {
                        forwarding.forward(range.start().raw(), len).is_none()
                    }) =>
                {
                    Some(*range)
                }
                _ => None,
            }
        }));
        for effect in &observed.committed_effects {
            let (range, bytes) = match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. }
                | cellgov_effects::Effect::ConditionalStore { range, bytes, .. } => (range, bytes),
                _ => continue,
            };
            let len = u8::try_from(range.length())
                .ok()
                .filter(|len| (1..=16).contains(len))
                .ok_or(PpuPathError::ForwardingWidth {
                    length: range.length(),
                })?;
            let value = bytes
                .bytes()
                .iter()
                .fold(0u128, |value, byte| (value << 8) | u128::from(*byte));
            match forwarding.insert(range.start().raw(), len, value) {
                Ok(()) => {}
                Err(cellgov_ppu::store_buffer::StoreRefusal::Full) => {
                    return Err(PpuPathError::ForwardingCapacity {
                        addr: range.start().raw(),
                    });
                }
                Err(source) => return Err(PpuPathError::ForwardingRefused { source }),
            }
        }
        data = observed.memory;
        reservations = reservation_table(&observed.reservations);
        effects_all.extend(
            observed
                .committed_effects
                .into_iter()
                .filter(|effect| !is_read_intent(effect)),
        );
        stop.pc = Some(step_pc);
        stop.diagnostics = LocalDiagnostics::with_pc(step_pc);
    }
    let mut observation = PpuObservation {
        state: PpuArchitecturalState::capture(&state),
        memory: data,
        outcome: PpuObservedOutcome::NoInstruction,
        staged_effects: effects_all.clone(),
        committed_effects: effects_all,
        commit_error: None,
        reservations: reservations.iter().collect(),
        store_buffer: Vec::new(),
        fault_discarded: stop.reason == YieldReason::Fault,
    };
    crate::seeded::ppu_observed(&mut observation);
    Ok(PpuPathRun {
        path: PpuExecutionPath::Plain,
        observation,
        stop,
        retired,
        executed_pcs,
        committed_data_reads,
    })
}

fn is_read_intent(effect: &cellgov_effects::Effect) -> bool {
    matches!(effect, cellgov_effects::Effect::SharedReadIntent { .. })
}

fn fault_diagnostics(state: &PpuState, pc: u64, ea: Option<u64>) -> LocalDiagnostics {
    let fields = PpuArchitecturalState::capture(state);
    LocalDiagnostics {
        pc: Some(pc),
        lr: Some(fields.lr),
        syscall_lev: None,
        faulting_ea: ea,
        fault_regs: Some(FaultRegisterDump {
            gprs: fields.gpr,
            lr: fields.lr,
            ctr: fields.ctr,
            xer: fields.xer,
            cr: fields.cr,
        }),
    }
}

fn refuse_commit(path: PpuExecutionPath, observed: &PpuObservation) -> Result<(), PpuPathError> {
    if let Some(error) = &observed.commit_error {
        return Err(PpuPathError::CommitRefusal {
            path,
            error: error.clone(),
            staged_effects: observed.staged_effects.clone(),
            committed_effects: observed.committed_effects.clone(),
        });
    }
    Ok(())
}

fn make_memory(code: &[u8], data: &[u8]) -> Result<GuestMemory, PpuPathError> {
    let code_size = code.len().max(4);
    let mut memory = GuestMemory::from_regions(vec![
        Region::new(0, code_size, "fuzz-code", PageSize::Page4K),
        Region::new(DATA_BASE, data.len(), "fuzz-data", PageSize::Page4K),
    ])?;
    let code_range = ByteRange::new(GuestAddr::new(0), code.len() as u64).ok_or(
        PpuPathError::RangeOverflow {
            base: 0,
            size: code.len() as u64,
        },
    )?;
    let data_range = ByteRange::new(GuestAddr::new(DATA_BASE), data.len() as u64).ok_or(
        PpuPathError::RangeOverflow {
            base: DATA_BASE,
            size: data.len() as u64,
        },
    )?;
    memory.apply_commit(code_range, code)?;
    memory.apply_commit(data_range, data)?;
    Ok(memory)
}

fn reservation_table(entries: &[(UnitId, ReservedLine)]) -> ReservationTable {
    let mut table = ReservationTable::new();
    for &(unit, line) in entries {
        table.insert_or_replace(unit, line);
    }
    table
}

fn initial_reservations(initial: &PpuState) -> ReservationTable {
    let mut table = ReservationTable::new();
    if let Some(line) = initial.reservation() {
        table.insert_or_replace(UNIT, line);
    }
    table
}

#[cfg(test)]
#[path = "tests/ppu_paths_tests.rs"]
mod tests;
