//! Interpreter-owned SPU observations and allowed write footprints.

use std::collections::BTreeSet;

use cellgov_effects::{Effect, EffectKind};

use crate::exec::SpuStepOutcome;
use crate::instruction::SpuInstruction;
use crate::state::{SpuChannelSnapshot, SpuObservableSnapshot, SpuState};

/// One SPU channel-state field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuChannelField {
    /// Staged local-store address.
    MfcLsa,
    /// Staged effective-address high word.
    MfcEah,
    /// Staged effective-address low word.
    MfcEal,
    /// Staged transfer size.
    MfcSize,
    /// Staged tag identifier.
    MfcTagId,
    /// Tag-query mask.
    TagMask,
    /// Completed tag status.
    TagStatus,
    /// Atomic-command result.
    AtomicStatus,
    /// Pending mailbox destination register.
    PendingMailbox,
    /// Pending DMA GET command.
    PendingGet,
}

fn channel_differences(
    before: &SpuChannelSnapshot,
    after: &SpuChannelSnapshot,
) -> BTreeSet<SpuChannelField> {
    let mut fields = BTreeSet::new();
    for (changed, field) in [
        (before.mfc_lsa != after.mfc_lsa, SpuChannelField::MfcLsa),
        (before.mfc_eah != after.mfc_eah, SpuChannelField::MfcEah),
        (before.mfc_eal != after.mfc_eal, SpuChannelField::MfcEal),
        (before.mfc_size != after.mfc_size, SpuChannelField::MfcSize),
        (
            before.mfc_tag_id != after.mfc_tag_id,
            SpuChannelField::MfcTagId,
        ),
        (before.tag_mask != after.tag_mask, SpuChannelField::TagMask),
        (
            before.tag_status != after.tag_status,
            SpuChannelField::TagStatus,
        ),
        (
            before.atomic_status != after.atomic_status,
            SpuChannelField::AtomicStatus,
        ),
        (
            before.pending_mbox_rt != after.pending_mbox_rt,
            SpuChannelField::PendingMailbox,
        ),
        (
            before.pending_get != after.pending_get,
            SpuChannelField::PendingGet,
        ),
    ] {
        if changed {
            fields.insert(field);
        }
    }
    fields
}

/// One independently comparable component of SPU execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuObservationComponent {
    /// Architectural register file.
    Registers,
    /// Local-store bytes.
    LocalStore,
    /// Program counter.
    ProgramCounter,
    /// Channel state, including pending mailbox and DMA work.
    Channels,
    /// Local reservation.
    Reservation,
    /// Typed executor outcome.
    Outcome,
    /// Ordered emitted effects.
    Effects,
    /// Fault-discard classification.
    FaultDiscard,
}

/// Complete SPU state, outcome, and effect observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuObservation {
    /// Architectural state after execution.
    pub state: SpuObservableSnapshot,
    /// Typed executor outcome.
    pub outcome: SpuStepOutcome,
    /// Effects emitted before the commit boundary.
    pub effects: Vec<Effect>,
    /// Whether the instruction returned a fault.
    pub fault_discarded: bool,
}

/// Typed differences between two SPU observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuObservationComparison {
    /// Every component that differs.
    pub differences: BTreeSet<SpuObservationComponent>,
}

impl SpuObservation {
    /// Captures the complete state and typed outcome after one instruction.
    // [Armstrong2019 p:71:1 s:Abstract] Executable ISA models can be used to validate architectural behavior.
    pub fn capture(state: &SpuState, outcome: &SpuStepOutcome) -> Self {
        Self::from_parts(SpuObservableSnapshot::capture(state), outcome.clone())
    }

    /// Combines a complete architectural snapshot with its typed executor outcome.
    pub fn from_parts(state: SpuObservableSnapshot, outcome: SpuStepOutcome) -> Self {
        let effects = match outcome {
            SpuStepOutcome::Yield { ref effects, .. } => effects.clone(),
            SpuStepOutcome::Continue
            | SpuStepOutcome::Branch
            | SpuStepOutcome::MemoryRead { .. }
            | SpuStepOutcome::Fault(_) => Vec::new(),
        };
        Self {
            state,
            fault_discarded: matches!(outcome, SpuStepOutcome::Fault(_)),
            outcome,
            effects,
        }
    }

    /// Compares every architectural and outcome component.
    pub fn compare(&self, other: &Self) -> SpuObservationComparison {
        let mut differences = BTreeSet::new();
        if self.state.regs != other.state.regs {
            differences.insert(SpuObservationComponent::Registers);
        }
        if self.state.ls != other.state.ls {
            differences.insert(SpuObservationComponent::LocalStore);
        }
        if self.state.pc != other.state.pc {
            differences.insert(SpuObservationComponent::ProgramCounter);
        }
        if self.state.channels != other.state.channels {
            differences.insert(SpuObservationComponent::Channels);
        }
        if self.state.reservation != other.state.reservation {
            differences.insert(SpuObservationComponent::Reservation);
        }
        if self.outcome != other.outcome {
            differences.insert(SpuObservationComponent::Outcome);
        }
        if self.effects != other.effects {
            differences.insert(SpuObservationComponent::Effects);
        }
        if self.fault_discarded != other.fault_discarded {
            differences.insert(SpuObservationComponent::FaultDiscard);
        }
        SpuObservationComparison { differences }
    }
}

/// Interpreter-owned allowed write and effect footprint for one instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuAllowedFootprint {
    /// Registers that the instruction may replace.
    pub registers: BTreeSet<u8>,
    /// Whether the instruction may replace local-store bytes.
    pub local_store: bool,
    /// Channel-state fields that the instruction may replace.
    pub channels: BTreeSet<SpuChannelField>,
    /// Whether the instruction may replace its reservation.
    pub reservation: bool,
    /// Whether the instruction may select a non-sequential program counter.
    pub control_transfer: bool,
    /// Effect classes declared by the instruction descriptor.
    pub effects: BTreeSet<EffectKind>,
}

impl SpuAllowedFootprint {
    /// Derives permitted writes from the decoded instruction and its effect contract.
    pub fn for_instruction(instruction: &SpuInstruction) -> Self {
        let mut footprint = Self {
            registers: BTreeSet::new(),
            local_store: false,
            channels: BTreeSet::new(),
            reservation: false,
            control_transfer: false,
            effects: instruction
                .fuzz_descriptor()
                .effects
                .iter()
                .copied()
                .collect(),
        };
        let register = match *instruction {
            SpuInstruction::Lqd { rt, .. }
            | SpuInstruction::Lqx { rt, .. }
            | SpuInstruction::Lqa { rt, .. }
            | SpuInstruction::Lqr { rt, .. }
            | SpuInstruction::Il { rt, .. }
            | SpuInstruction::Ila { rt, .. }
            | SpuInstruction::Ilh { rt, .. }
            | SpuInstruction::Ilhu { rt, .. }
            | SpuInstruction::Iohl { rt, .. }
            | SpuInstruction::Fsmbi { rt, .. }
            | SpuInstruction::A { rt, .. }
            | SpuInstruction::Ai { rt, .. }
            | SpuInstruction::Sf { rt, .. }
            | SpuInstruction::And { rt, .. }
            | SpuInstruction::Or { rt, .. }
            | SpuInstruction::Selb { rt, .. }
            | SpuInstruction::Xsbh { rt, .. }
            | SpuInstruction::Gb { rt, .. }
            | SpuInstruction::Gbh { rt, .. }
            | SpuInstruction::Ori { rt, .. }
            | SpuInstruction::Nor { rt, .. }
            | SpuInstruction::Andi { rt, .. }
            | SpuInstruction::Shufb { rt, .. }
            | SpuInstruction::Shlqbyi { rt, .. }
            | SpuInstruction::Rotqby { rt, .. }
            | SpuInstruction::Rotqbyi { rt, .. }
            | SpuInstruction::Rotqmbyi { rt, .. }
            | SpuInstruction::Shl { rt, .. }
            | SpuInstruction::Shli { rt, .. }
            | SpuInstruction::Rotmi { rt, .. }
            | SpuInstruction::Rotmai { rt, .. }
            | SpuInstruction::Cbd { rt, .. }
            | SpuInstruction::Cbx { rt, .. }
            | SpuInstruction::Chd { rt, .. }
            | SpuInstruction::Chx { rt, .. }
            | SpuInstruction::Cwd { rt, .. }
            | SpuInstruction::Cwx { rt, .. }
            | SpuInstruction::Cdd { rt, .. }
            | SpuInstruction::Cdx { rt, .. }
            | SpuInstruction::Ceq { rt, .. }
            | SpuInstruction::Ceqi { rt, .. }
            | SpuInstruction::Ceqbi { rt, .. }
            | SpuInstruction::Cgti { rt, .. }
            | SpuInstruction::Clgt { rt, .. }
            | SpuInstruction::Brsl { rt, .. }
            | SpuInstruction::Bisl { rt, .. }
            | SpuInstruction::Rchcnt { rt, .. } => Some(rt),
            // [CBE-Handbook p:542 s:19.6.6.3 SPU Side] An empty inbound mailbox stalls the read.
            // The executor writes RT only when a message arrives on re-entry.
            SpuInstruction::Rdch {
                channel: cellgov_ps3_abi::hw::spu::SPU_RD_IN_MBOX,
                ..
            } => None,
            SpuInstruction::Rdch { rt, .. } => Some(rt),
            SpuInstruction::Stqd { .. }
            | SpuInstruction::Stqx { .. }
            | SpuInstruction::Stqa { .. }
            | SpuInstruction::Stqr { .. } => {
                footprint.local_store = true;
                None
            }
            SpuInstruction::Wrch { channel, .. } => {
                use cellgov_ps3_abi::hw::spu;
                let field = match channel {
                    spu::MFC_LSA => Some(SpuChannelField::MfcLsa),
                    spu::MFC_EAH => Some(SpuChannelField::MfcEah),
                    spu::MFC_EAL => Some(SpuChannelField::MfcEal),
                    spu::MFC_SIZE => Some(SpuChannelField::MfcSize),
                    spu::MFC_TAG_ID => Some(SpuChannelField::MfcTagId),
                    spu::MFC_WR_TAG_MASK => Some(SpuChannelField::TagMask),
                    spu::MFC_CMD => Some(SpuChannelField::PendingGet),
                    spu::MFC_WR_TAG_UPDATE | spu::SPU_WR_OUT_MBOX | spu::SPU_WR_OUT_INTR_MBOX => {
                        None
                    }
                    _ => None,
                };
                if let Some(field) = field {
                    footprint.channels.insert(field);
                }
                if channel == spu::MFC_CMD {
                    footprint.channels.insert(SpuChannelField::AtomicStatus);
                    footprint.reservation = true;
                }
                None
            }
            SpuInstruction::Br { .. }
            | SpuInstruction::Brz { .. }
            | SpuInstruction::Brnz { .. }
            | SpuInstruction::Bi { .. }
            | SpuInstruction::Brhnz { .. }
            | SpuInstruction::Brhz { .. }
            | SpuInstruction::Biz { .. }
            | SpuInstruction::Binz { .. }
            | SpuInstruction::Bihz { .. }
            | SpuInstruction::Bihnz { .. }
            | SpuInstruction::Nop
            | SpuInstruction::Lnop
            | SpuInstruction::Hbr
            | SpuInstruction::Hbra
            | SpuInstruction::Hbrr
            | SpuInstruction::Sync
            | SpuInstruction::Dsync
            | SpuInstruction::Heq
            | SpuInstruction::Stop { .. } => None,
        };
        if let Some(register) = register {
            footprint.registers.insert(register);
        }
        footprint.control_transfer = matches!(
            instruction,
            SpuInstruction::Br { .. }
                | SpuInstruction::Brsl { .. }
                | SpuInstruction::Brz { .. }
                | SpuInstruction::Brnz { .. }
                | SpuInstruction::Bi { .. }
                | SpuInstruction::Bisl { .. }
                | SpuInstruction::Brhnz { .. }
                | SpuInstruction::Brhz { .. }
                | SpuInstruction::Biz { .. }
                | SpuInstruction::Binz { .. }
                | SpuInstruction::Bihz { .. }
                | SpuInstruction::Bihnz { .. }
        );
        if matches!(
            instruction,
            SpuInstruction::Rdch {
                channel: cellgov_ps3_abi::hw::spu::SPU_RD_IN_MBOX,
                ..
            }
        ) {
            footprint.channels.insert(SpuChannelField::PendingMailbox);
        }
        footprint
    }

    /// Names state or effect changes outside this instruction's declared footprint.
    pub fn violations(
        &self,
        initial: &SpuState,
        observed: &SpuObservation,
    ) -> BTreeSet<SpuObservationComponent> {
        let before = SpuObservableSnapshot::capture(initial);
        let mut violations = BTreeSet::new();
        if observed.fault_discarded {
            if before.regs != observed.state.regs {
                violations.insert(SpuObservationComponent::Registers);
            }
            if before.ls != observed.state.ls {
                violations.insert(SpuObservationComponent::LocalStore);
            }
            if before.pc != observed.state.pc {
                violations.insert(SpuObservationComponent::ProgramCounter);
            }
            if before.channels != observed.state.channels {
                violations.insert(SpuObservationComponent::Channels);
            }
            if before.reservation != observed.state.reservation {
                violations.insert(SpuObservationComponent::Reservation);
            }
        } else {
            for (index, (previous, current)) in
                before.regs.iter().zip(&observed.state.regs).enumerate()
            {
                // The executor's yielding arms do not publish register values.
                if previous != current
                    && (matches!(&observed.outcome, SpuStepOutcome::Yield { .. })
                        || !self.registers.contains(&(index as u8)))
                {
                    violations.insert(SpuObservationComponent::Registers);
                }
            }
            if before.ls != observed.state.ls && !self.local_store {
                violations.insert(SpuObservationComponent::LocalStore);
            }
            if before.pc != observed.state.pc && !self.control_transfer {
                violations.insert(SpuObservationComponent::ProgramCounter);
            }
            if !channel_differences(&before.channels, &observed.state.channels)
                .is_subset(&self.channels)
            {
                violations.insert(SpuObservationComponent::Channels);
            }
            if before.reservation != observed.state.reservation && !self.reservation {
                violations.insert(SpuObservationComponent::Reservation);
            }
        }
        // A fault discards effects even when their kind is otherwise allowed.
        if (observed.fault_discarded && !observed.effects.is_empty())
            || observed
                .effects
                .iter()
                .any(|effect| !self.effects.contains(&effect.kind()))
        {
            violations.insert(SpuObservationComponent::Effects);
        }
        violations
    }
}
