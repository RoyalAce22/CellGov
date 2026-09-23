//! Runs internal PPU paths for differential optimization-consistency checks. Agreement does not establish hardware accuracy.

use std::collections::BTreeSet;

use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
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
}

/// Compares internal paths with the same instruction sequence, PPU state, and data.
// [McKeeman1998 p:100 s:Abstract] Comparable systems receive identical generated tests.
pub fn run_all_paths(
    words: &[u32],
    initial: &PpuState,
    data: &[u8],
) -> Result<Vec<PpuPathRun>, PpuPathError> {
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
    [
        PpuExecutionPath::Plain,
        PpuExecutionPath::Forwarded,
        PpuExecutionPath::Quickened,
        PpuExecutionPath::Fused,
    ]
    .into_iter()
    .map(|path| run_path(path, &code, words.len(), initial, data))
    .collect()
}

/// Scans runs in caller order and returns their first disagreement.
pub fn first_path_divergence(runs: &[PpuPathRun]) -> Option<PpuPathDivergence> {
    for (index, left) in runs.iter().enumerate() {
        for right in runs.iter().skip(index + 1) {
            let comparison = left
                .observation
                .compare(&right.observation, PpuObservationCheck::DeterministicReplay);
            let stop_differs = left.stop != right.stop;
            let retired_differs = left.retired != right.retired;
            if !comparison.complete_differences.is_empty() || stop_differs || retired_differs {
                return Some(PpuPathDivergence {
                    left: left.path,
                    right: right.path,
                    observation: comparison.complete_differences,
                    stop_differs,
                    retired_differs,
                });
            }
        }
    }
    None
}

fn run_path(
    path: PpuExecutionPath,
    code: &[u8],
    words: usize,
    initial: &PpuState,
    initial_data: &[u8],
) -> Result<PpuPathRun, PpuPathError> {
    if path == PpuExecutionPath::Plain {
        return run_plain(code, words, initial, initial_data);
    }
    let budget =
        Budget::new(u64::try_from(words).map_err(|_| PpuPathError::BudgetOverflow { words })?);
    let mut unit = PpuExecutionUnit::new(UNIT);
    *unit.state_mut() = initial.clone();
    match path {
        PpuExecutionPath::Quickened => {
            unit.set_instruction_shadow(PredecodedShadow::build_quickened(0, code));
        }
        PpuExecutionPath::Fused => {
            unit.set_instruction_shadow(PredecodedShadow::build(0, code));
        }
        PpuExecutionPath::Plain | PpuExecutionPath::Forwarded => {}
    }

    let mut data = initial_data.to_vec();
    let mut reservations = initial_reservations(initial);
    let mut effects_all = Vec::new();
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
    };
    Ok(PpuPathRun {
        path,
        observation: PpuObservation {
            state: PpuArchitecturalState::capture(unit.state()),
            memory: data,
            outcome: PpuObservedOutcome::NoInstruction,
            staged_effects: effects_all.clone(),
            committed_effects: effects_all,
            commit_error: None,
            reservations: reservations.iter().collect(),
            store_buffer: Vec::new(),
            fault_discarded: stop.reason == YieldReason::Fault,
        },
        stop,
        retired,
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
    let mut retired = 0u64;
    let mut stop = PpuPathStop {
        reason: YieldReason::BudgetExhausted,
        fault: None,
        pc: Some(0),
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
            state = initial.clone();
            data.copy_from_slice(initial_data);
            reservations = initial_reservations(initial);
            effects_all.clear();
            retired = 0;
            stop = PpuPathStop {
                reason: YieldReason::Fault,
                fault: Some(FaultKind::Guest(cellgov_ppu::FAULT_DECODE_ERROR)),
                pc: Some(step_pc),
            };
            break;
        };
        let entry = state.clone();
        let mut effects = Vec::new();
        let mut stores = StoreBuffer::new();
        let views = [RegionView::plain(DATA_BASE, &data)];
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
                state = initial.clone();
                data.copy_from_slice(initial_data);
                reservations = initial_reservations(initial);
                effects_all.clear();
                retired = 0;
                stop = PpuPathStop {
                    reason: YieldReason::Fault,
                    fault: Some(FaultKind::Guest(fault.guest_code())),
                    pc: Some(step_pc),
                };
                break;
            }
            ExecuteVerdict::MemFault(_) => {
                state = initial.clone();
                data.copy_from_slice(initial_data);
                reservations = initial_reservations(initial);
                effects_all.clear();
                retired = 0;
                stop = PpuPathStop {
                    reason: YieldReason::Fault,
                    fault: Some(FaultKind::Guest(cellgov_ppu::FAULT_INVALID_ADDRESS)),
                    pc: Some(step_pc),
                };
                break;
            }
            ExecuteVerdict::Syscall { .. } => {
                stop = PpuPathStop {
                    reason: YieldReason::Syscall,
                    fault: None,
                    pc: Some(step_pc),
                };
                break;
            }
            ExecuteVerdict::BufferFull => {
                stop.pc = Some(step_pc);
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
        data = observed.memory;
        reservations = reservation_table(&observed.reservations);
        effects_all.extend(
            observed
                .committed_effects
                .into_iter()
                .filter(|effect| !is_read_intent(effect)),
        );
        stop.pc = Some(step_pc);
    }
    Ok(PpuPathRun {
        path: PpuExecutionPath::Plain,
        observation: PpuObservation {
            state: PpuArchitecturalState::capture(&state),
            memory: data,
            outcome: PpuObservedOutcome::NoInstruction,
            staged_effects: effects_all.clone(),
            committed_effects: effects_all,
            commit_error: None,
            reservations: reservations.iter().collect(),
            store_buffer: Vec::new(),
            fault_discarded: stop.reason == YieldReason::Fault,
        },
        stop,
        retired,
    })
}

fn is_read_intent(effect: &cellgov_effects::Effect) -> bool {
    matches!(effect, cellgov_effects::Effect::SharedReadIntent { .. })
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
