//! Complete PPU observations for differential and metamorphic checks.

use std::collections::BTreeSet;

use cellgov_effects::{Effect, EffectKind};
use cellgov_event::UnitId;
use cellgov_sync::{ReservationTable, ReservedLine};

use crate::exec::ExecuteVerdict;
use crate::instruction::fuzz::PpuPermittedDelta;
use crate::state::PpuState;
use crate::store_buffer::{PendingStore, StoreBuffer};

/// Architecturally visible PPU state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuArchitecturalState {
    /// General-purpose registers.
    pub gpr: [u64; 32],
    /// Floating-point registers as raw bit patterns.
    pub fpr: [u64; 32],
    /// Vector registers.
    pub vr: [u128; 32],
    /// Program counter.
    pub pc: u64,
    /// Condition register.
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// AltiVec usage mask.
    pub vrsave: u32,
    /// Time-base register.
    pub tb: u64,
    /// Local reservation register.
    pub reservation: Option<ReservedLine>,
}

impl PpuArchitecturalState {
    /// Capture every architecturally visible field in [`PpuState`].
    pub fn capture(state: &PpuState) -> Self {
        let fields = state.observation_fields();
        Self {
            gpr: fields.gpr,
            fpr: fields.fpr,
            vr: fields.vr,
            pc: fields.pc,
            cr: fields.cr,
            lr: fields.lr,
            ctr: fields.ctr,
            xer: fields.xer,
            vrsave: fields.vrsave,
            tb: fields.tb,
            reservation: fields.reservation,
        }
    }
}

/// Terminal result attached to a PPU observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuObservedOutcome {
    /// A decoded instruction produced this executor verdict.
    Execution(ExecuteVerdict),
    /// Fetch reached a word the decoder refused.
    DecodeRefusal {
        /// Address of the refused word.
        pc: u64,
        /// Refused instruction word.
        raw: u32,
        /// Last executor verdict, when an earlier instruction retired.
        prior: Option<ExecuteVerdict>,
    },
    /// The sequence ended before it fetched an instruction.
    NoInstruction,
    /// One execution-unit batch result.
    RuntimeStep(Box<cellgov_exec::ExecutionStepResult>),
}

impl PpuObservedOutcome {
    fn discards_batch(&self) -> bool {
        match self {
            Self::Execution(ExecuteVerdict::Fault(_) | ExecuteVerdict::MemFault(_)) => true,
            Self::Execution(
                ExecuteVerdict::Continue
                | ExecuteVerdict::Branch
                | ExecuteVerdict::Syscall { .. }
                | ExecuteVerdict::BufferFull,
            )
            | Self::NoInstruction => false,
            // `PpuExecutionUnit::run_batch` routes a decoder refusal through
            // `fault_yield`, which discards the whole atomic batch.
            Self::DecodeRefusal { .. } => true,
            Self::RuntimeStep(result) => result.yield_reason == cellgov_exec::YieldReason::Fault,
        }
    }

    fn effects_include_closed_block_metadata(&self) -> bool {
        matches!(self, Self::RuntimeStep(_))
    }
}

/// One independently comparable part of a complete PPU observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuObservationComponent {
    /// Architectural registers and local reservation.
    State,
    /// Memory after the effect batch commits.
    Memory,
    /// Terminal execution or decode result.
    Outcome,
    /// Effects emitted before the commit boundary.
    StagedEffects,
    /// Effects accepted by the commit boundary.
    CommittedEffects,
    /// Runtime reservation table after commit.
    Reservations,
    /// Stores pending before the block-boundary flush.
    StoreBuffer,
    /// A fault discarded the batch.
    FaultDiscard,
}

/// A named rule selecting the parts relevant to one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuObservationCheck {
    /// Compare every observable component for deterministic replay.
    DeterministicReplay,
    /// Compare architectural state and committed memory.
    ArchitecturalState,
    /// Compare terminal outcomes only.
    Outcome,
    /// Compare staged and committed effect streams.
    Effects,
}

impl PpuObservationCheck {
    fn includes(self, component: PpuObservationComponent) -> bool {
        match self {
            Self::DeterministicReplay => true,
            Self::ArchitecturalState => matches!(
                component,
                PpuObservationComponent::State
                    | PpuObservationComponent::Memory
                    | PpuObservationComponent::Reservations
                    | PpuObservationComponent::StoreBuffer
                    | PpuObservationComponent::FaultDiscard
            ),
            Self::Outcome => component == PpuObservationComponent::Outcome,
            Self::Effects => matches!(
                component,
                PpuObservationComponent::StagedEffects | PpuObservationComponent::CommittedEffects
            ),
        }
    }
}

/// Comparison results for two PPU observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuObservationComparison {
    /// Every component that differs before masking.
    pub complete_differences: BTreeSet<PpuObservationComponent>,
    /// Differences that the named check selects.
    pub relevant_differences: BTreeSet<PpuObservationComponent>,
}

/// Differences outside one metamorphic relation's permitted delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuMetamorphicComparison {
    /// Complete component-level differences before the relation mask.
    pub complete_differences: BTreeSet<PpuObservationComponent>,
    /// Components that the relation does not permit to change.
    pub disallowed_differences: BTreeSet<PpuObservationComponent>,
}

/// Complete PPU state at one execution boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuObservation {
    /// Architectural state after commit or fault rollback.
    pub state: PpuArchitecturalState,
    /// Bytes after committed writes, over the observed input region.
    pub memory: Vec<u8>,
    /// Terminal result.
    pub outcome: PpuObservedOutcome,
    /// Effects emitted before fault discard or store-buffer flush.
    pub staged_effects: Vec<Effect>,
    /// Effects accepted in emission order.
    pub committed_effects: Vec<Effect>,
    /// Commit validation refusal, when the emitted batch was not accepted.
    pub commit_error: Option<PpuObservationError>,
    /// Runtime reservation table after commit, in unit-id order.
    pub reservations: Vec<(UnitId, ReservedLine)>,
    /// Stores pending immediately before the block-boundary flush.
    pub store_buffer: Vec<PendingStore>,
    /// Whether the terminal fault discarded the atomic batch.
    pub fault_discarded: bool,
}

impl PpuObservation {
    /// Compare the complete observations, then apply a named mask.
    pub fn compare(&self, other: &Self, check: PpuObservationCheck) -> PpuObservationComparison {
        let mut complete = BTreeSet::new();
        if self.state != other.state {
            complete.insert(PpuObservationComponent::State);
        }
        if self.memory != other.memory {
            complete.insert(PpuObservationComponent::Memory);
        }
        if self.outcome != other.outcome {
            complete.insert(PpuObservationComponent::Outcome);
        }
        if self.staged_effects != other.staged_effects {
            complete.insert(PpuObservationComponent::StagedEffects);
        }
        if self.committed_effects != other.committed_effects {
            complete.insert(PpuObservationComponent::CommittedEffects);
        }
        if self.commit_error != other.commit_error {
            complete.insert(PpuObservationComponent::Outcome);
        }
        if self.reservations != other.reservations {
            complete.insert(PpuObservationComponent::Reservations);
        }
        if self.store_buffer != other.store_buffer {
            complete.insert(PpuObservationComponent::StoreBuffer);
        }
        if self.fault_discarded != other.fault_discarded {
            complete.insert(PpuObservationComponent::FaultDiscard);
        }
        let relevant = complete
            .iter()
            .copied()
            .filter(|component| check.includes(*component))
            .collect();
        PpuObservationComparison {
            complete_differences: complete,
            relevant_differences: relevant,
        }
    }

    /// Compares complete observations under one typed permitted delta.
    pub fn compare_metamorphic(
        &self,
        other: &Self,
        permitted: PpuPermittedDelta,
    ) -> PpuMetamorphicComparison {
        let comparison = self.compare(other, PpuObservationCheck::DeterministicReplay);
        let mut disallowed = comparison.complete_differences.clone();
        if !state_differs_outside(&self.state, &other.state, permitted) {
            disallowed.remove(&PpuObservationComponent::State);
        }
        PpuMetamorphicComparison {
            complete_differences: comparison.complete_differences,
            disallowed_differences: disallowed,
        }
    }
}

fn state_differs_outside(
    first: &PpuArchitecturalState,
    second: &PpuArchitecturalState,
    permitted: PpuPermittedDelta,
) -> bool {
    let (first_cr, second_cr, first_xer, second_xer) = match permitted {
        PpuPermittedDelta::Cr0 => masked_state_fields(first, second, 0),
        PpuPermittedDelta::Cr1 => masked_state_fields(first, second, 1),
        PpuPermittedDelta::Cr6 => masked_state_fields(first, second, 6),
        PpuPermittedDelta::XerOverflow => {
            let mask = !((1u64 << 30) | (1u64 << 31));
            (first.cr, second.cr, first.xer & mask, second.xer & mask)
        }
    };
    first.gpr != second.gpr
        || first.fpr != second.fpr
        || first.vr != second.vr
        || first.pc != second.pc
        || first_cr != second_cr
        || first.lr != second.lr
        || first.ctr != second.ctr
        || first_xer != second_xer
        || first.vrsave != second.vrsave
        || first.tb != second.tb
        || first.reservation != second.reservation
}

fn masked_state_fields(
    first: &PpuArchitecturalState,
    second: &PpuArchitecturalState,
    field: u8,
) -> (u32, u32, u64, u64) {
    let shift = u32::from(7 - field) * 4;
    let mask = !(0x0fu32 << shift);
    (first.cr & mask, second.cr & mask, first.xer, second.xer)
}

/// Why a PPU observation could not apply its emitted effect batch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpuObservationError {
    /// A committed write escaped the observed memory region.
    #[error("{kind:?} write at 0x{addr:016x} (length {len}) escapes the observed memory")]
    WriteOutOfRange {
        /// Effect class carrying the write.
        kind: EffectKind,
        /// First guest byte.
        addr: u64,
        /// Write width.
        len: u64,
    },
    /// A write payload length disagreed with its range.
    #[error("{kind:?} payload length disagrees with its range")]
    PayloadLengthMismatch {
        /// Effect class carrying the write.
        kind: EffectKind,
    },
    /// An effect that a PPU execution cannot emit reached this contract.
    #[error("PPU observation received unsupported effect {kind:?}")]
    UnsupportedEffect {
        /// Unexpected effect class.
        kind: EffectKind,
    },
    /// A fault packet reached a non-faulting terminal outcome.
    #[error("FaultRaised reached a non-faulting PPU observation")]
    FaultEffectWithoutFault,
}

/// Inputs needed to finish one PPU execution batch.
pub struct PpuObservationInput<'a> {
    /// Architectural state at batch entry.
    pub initial_state: &'a PpuState,
    /// Architectural state after execution.
    pub final_state: &'a PpuState,
    /// Guest base address of `initial_memory`.
    pub memory_base: u64,
    /// Committed memory at batch entry.
    pub initial_memory: &'a [u8],
    /// Terminal executor or decoder result.
    pub outcome: PpuObservedOutcome,
    /// Effects emitted before the store-buffer flush.
    pub effects: Vec<Effect>,
    /// Stores waiting at the block boundary.
    pub stores: StoreBuffer,
    /// PPU unit that executes the batch.
    pub unit: UnitId,
}

/// Finish one PPU batch through the interpreter-owned observation contract.
// [Armstrong2019 p:71:1 s:Abstract] The observation is derived beside the executable ISA state.
pub fn finish_observation(
    input: PpuObservationInput<'_>,
) -> Result<PpuObservation, PpuObservationError> {
    let PpuObservationInput {
        initial_state,
        final_state,
        memory_base,
        initial_memory,
        outcome,
        mut effects,
        mut stores,
        unit,
    } = input;
    let store_buffer = stores.snapshot();
    let fault_discarded = outcome.discards_batch();
    let conditional_store_executed = initial_state.stdcx_executed != final_state.stdcx_executed
        || initial_state.stwcx_executed != final_state.stwcx_executed;
    let staged_effects = if !fault_discarded {
        stores.flush(&mut effects, unit);
        if final_state.clock_read && !outcome.effects_include_closed_block_metadata() {
            effects.push(Effect::ClockRead { source: unit });
        }
        effects.clone()
    } else {
        let staged = effects.clone();
        effects.clear();
        staged
    };
    let mut memory = initial_memory.to_vec();
    let mut reservations = ReservationTable::new();
    if let Some(line) = initial_state.reservation() {
        reservations.insert_or_replace(unit, line);
    }
    let commit_error =
        match apply_committed_effects(memory_base, &mut memory, &effects, &mut reservations) {
            Ok(()) => None,
            Err(
                error @ (PpuObservationError::WriteOutOfRange { .. }
                | PpuObservationError::PayloadLengthMismatch { .. }),
            ) => {
                memory.copy_from_slice(initial_memory);
                reservations = ReservationTable::new();
                if let Some(line) = initial_state.reservation() {
                    reservations.insert_or_replace(unit, line);
                }
                Some(error)
            }
            Err(error) => return Err(error),
        };
    // [PPC-Book2 p:25 s:3.3] A store-conditional attempt clears its reservation even when it stores nothing.
    if !fault_discarded
        && commit_error.is_none()
        && conditional_store_executed
        && final_state.reservation().is_none()
    {
        reservations.remove_if_present(unit);
    }
    let committed_effects = if fault_discarded || commit_error.is_some() {
        Vec::new()
    } else {
        effects
    };

    Ok(PpuObservation {
        state: PpuArchitecturalState::capture(if fault_discarded {
            initial_state
        } else {
            final_state
        }),
        memory,
        outcome,
        staged_effects,
        committed_effects,
        commit_error,
        reservations: reservations.iter().collect(),
        store_buffer,
        fault_discarded,
    })
}

fn apply_committed_effects(
    memory_base: u64,
    memory: &mut [u8],
    effects: &[Effect],
    reservations: &mut ReservationTable,
) -> Result<(), PpuObservationError> {
    for effect in effects {
        match effect {
            Effect::SharedWriteIntent {
                range,
                bytes,
                source,
                ..
            } => {
                apply_write(memory_base, memory, effect.kind(), *range, bytes.bytes())?;
                reservations.clear_covering(range.start().raw(), range.length(), Some(*source));
            }
            Effect::ConditionalStore {
                range,
                bytes,
                source,
                ..
            } => {
                apply_write(memory_base, memory, effect.kind(), *range, bytes.bytes())?;
                reservations.remove_if_present(*source);
                reservations.clear_covering(range.start().raw(), range.length(), None);
            }
            Effect::ReservationAcquire { line_addr, source } => {
                reservations.insert_or_replace(*source, ReservedLine::containing(*line_addr));
            }
            Effect::SharedReadIntent { .. }
            | Effect::ClockRead { .. }
            | Effect::TraceMarker { .. } => {}
            Effect::FaultRaised { .. } => return Err(PpuObservationError::FaultEffectWithoutFault),
            Effect::MailboxSend { .. }
            | Effect::MailboxReceiveAttempt { .. }
            | Effect::DmaEnqueue { .. }
            | Effect::WaitOnEvent { .. }
            | Effect::WakeUnit { .. }
            | Effect::SignalUpdate { .. }
            | Effect::RsxLabelWrite { .. }
            | Effect::RsxFlipRequest { .. } => {
                return Err(PpuObservationError::UnsupportedEffect {
                    kind: effect.kind(),
                });
            }
        }
    }
    Ok(())
}

fn apply_write(
    memory_base: u64,
    memory: &mut [u8],
    kind: EffectKind,
    range: cellgov_mem::ByteRange,
    bytes: &[u8],
) -> Result<(), PpuObservationError> {
    if range.length() != bytes.len() as u64 {
        return Err(PpuObservationError::PayloadLengthMismatch { kind });
    }
    let Some(offset) = range.start().raw().checked_sub(memory_base) else {
        return Err(PpuObservationError::WriteOutOfRange {
            kind,
            addr: range.start().raw(),
            len: range.length(),
        });
    };
    let Some(end) = offset.checked_add(range.length()) else {
        return Err(PpuObservationError::WriteOutOfRange {
            kind,
            addr: range.start().raw(),
            len: range.length(),
        });
    };
    let Ok(offset) = usize::try_from(offset) else {
        return Err(PpuObservationError::WriteOutOfRange {
            kind,
            addr: range.start().raw(),
            len: range.length(),
        });
    };
    let Ok(end) = usize::try_from(end) else {
        return Err(PpuObservationError::WriteOutOfRange {
            kind,
            addr: range.start().raw(),
            len: range.length(),
        });
    };
    let Some(destination) = memory.get_mut(offset..end) else {
        return Err(PpuObservationError::WriteOutOfRange {
            kind,
            addr: range.start().raw(),
            len: range.length(),
        });
    };
    destination.copy_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
#[path = "tests/observation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/metamorphic_observation_tests.rs"]
mod metamorphic_tests;
