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
    /// Inbound mailbox messages the step has not read.
    InboundMailbox,
    /// Free MFC command-queue slots.
    CommandQueue,
    /// Waiting tag-status update request.
    TagUpdate,
    /// Latched tag status not yet read.
    TagStatusRead,
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
            before.in_mbox != after.in_mbox,
            SpuChannelField::InboundMailbox,
        ),
        (
            before.cmd_queue_free != after.cmd_queue_free,
            SpuChannelField::CommandQueue,
        ),
        (
            before.tag_update != after.tag_update,
            SpuChannelField::TagUpdate,
        ),
        (
            before.tag_status_read != after.tag_status_read,
            SpuChannelField::TagStatusRead,
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
    /// Floating-point status and control register.
    Fpscr,
    /// Signal-notification registers.
    Signals,
    /// Interrupt-enable state and SRR0.
    Interrupts,
}

/// Complete SPU state, outcome, and effect observation.
///
/// [Wang2024 p:340:17 s:3.9] A run must expose its final state, or a divergence between two runs cannot be seen.
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
    ///
    /// [Armstrong2019 p:71:23 s:7] Validation of an executable ISA model runs the model and checks the behaviour it exhibits.
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
            | SpuStepOutcome::Fault(_)
            | SpuStepOutcome::Stop { .. } => Vec::new(),
        };
        Self {
            state,
            fault_discarded: matches!(outcome, SpuStepOutcome::Fault(_)),
            outcome,
            effects,
        }
    }

    /// Compares every architectural and outcome component.
    ///
    /// [Martignoni2009 p:127 s:2.2] Two executions agree only when the program counter, registers, memory and exception state all match afterwards.
    pub fn compare(&self, other: &Self) -> SpuObservationComparison {
        let mut differences = state_differences(&self.state, &other.state);
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

/// The architectural components on which two snapshots differ.
///
/// Every field is named, so a new one fails to compile here until it is
/// compared or marked as not a component.
fn state_differences(
    a: &SpuObservableSnapshot,
    b: &SpuObservableSnapshot,
) -> BTreeSet<SpuObservationComponent> {
    let SpuObservableSnapshot {
        regs,
        ls,
        pc,
        // Not components of an instruction observation.
        lslr: _,
        stop: _,
        signals,
        channels,
        reservation,
        fpscr,
        interrupts_enabled,
        srr0,
    } = a;
    [
        (*regs != b.regs, SpuObservationComponent::Registers),
        (*ls != b.ls, SpuObservationComponent::LocalStore),
        (*pc != b.pc, SpuObservationComponent::ProgramCounter),
        (*channels != b.channels, SpuObservationComponent::Channels),
        (
            *reservation != b.reservation,
            SpuObservationComponent::Reservation,
        ),
        (*fpscr != b.fpscr, SpuObservationComponent::Fpscr),
        (*signals != b.signals, SpuObservationComponent::Signals),
        (
            (*interrupts_enabled, *srr0) != (b.interrupts_enabled, b.srr0),
            SpuObservationComponent::Interrupts,
        ),
    ]
    .into_iter()
    .filter_map(|(differs, component)| differs.then_some(component))
    .collect()
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
    /// Whether the instruction may replace FPSCR bits.
    pub fpscr: bool,
    /// Whether the instruction may clear a signal-notification register.
    pub signals: bool,
    /// Whether the instruction may change the interrupt-enable state or
    /// SRR0.
    pub interrupts: bool,
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
            interrupts: matches!(
                instruction,
                SpuInstruction::Bi { .. }
                    | SpuInstruction::Bisl { .. }
                    | SpuInstruction::Bisled { .. }
                    | SpuInstruction::Biz { .. }
                    | SpuInstruction::Binz { .. }
                    | SpuInstruction::Bihz { .. }
                    | SpuInstruction::Bihnz { .. }
                    | SpuInstruction::Iret { .. }
                    | SpuInstruction::Wrch {
                        channel: cellgov_ps3_abi::hw::spu::SPU_WR_SRR0,
                        ..
                    }
            ),
            signals: matches!(
                instruction,
                SpuInstruction::Rdch {
                    channel: cellgov_ps3_abi::hw::spu::SPU_RD_SIG_NOTIFY_1
                        | cellgov_ps3_abi::hw::spu::SPU_RD_SIG_NOTIFY_2,
                    ..
                }
            ),
            fpscr: matches!(
                instruction,
                SpuInstruction::Fscrwr { .. }
                    | SpuInstruction::Fa { .. }
                    | SpuInstruction::Fs { .. }
                    | SpuInstruction::Fm { .. }
                    | SpuInstruction::Fma { .. }
                    | SpuInstruction::Fms { .. }
                    | SpuInstruction::Fnms { .. }
                    | SpuInstruction::Frest { .. }
                    | SpuInstruction::Frsqest { .. }
                    | SpuInstruction::Fi { .. }
                    | SpuInstruction::Csflt { .. }
                    | SpuInstruction::Cuflt { .. }
                    | SpuInstruction::Dfa { .. }
                    | SpuInstruction::Dfs { .. }
                    | SpuInstruction::Dfm { .. }
                    | SpuInstruction::Dfma { .. }
                    | SpuInstruction::Dfms { .. }
                    | SpuInstruction::Dfnms { .. }
                    | SpuInstruction::Dfnma { .. }
                    | SpuInstruction::Frds { .. }
                    | SpuInstruction::Fesd { .. }
            ),
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
            | SpuInstruction::Avgb { rt, .. }
            | SpuInstruction::Absdb { rt, .. }
            | SpuInstruction::Sumb { rt, .. }
            | SpuInstruction::Mpy { rt, .. }
            | SpuInstruction::Mpyu { rt, .. }
            | SpuInstruction::Mpyi { rt, .. }
            | SpuInstruction::Mpyui { rt, .. }
            | SpuInstruction::Mpya { rt, .. }
            | SpuInstruction::Mpyh { rt, .. }
            | SpuInstruction::Mpys { rt, .. }
            | SpuInstruction::Mpyhh { rt, .. }
            | SpuInstruction::Mpyhha { rt, .. }
            | SpuInstruction::Mpyhhu { rt, .. }
            | SpuInstruction::Mpyhhau { rt, .. }
            | SpuInstruction::Addx { rt, .. }
            | SpuInstruction::Cg { rt, .. }
            | SpuInstruction::Cgx { rt, .. }
            | SpuInstruction::Sfx { rt, .. }
            | SpuInstruction::Bg { rt, .. }
            | SpuInstruction::Bgx { rt, .. }
            | SpuInstruction::Ah { rt, .. }
            | SpuInstruction::Ahi { rt, .. }
            | SpuInstruction::Sfh { rt, .. }
            | SpuInstruction::Sfhi { rt, .. }
            | SpuInstruction::Sfi { rt, .. }
            | SpuInstruction::And { rt, .. }
            | SpuInstruction::Or { rt, .. }
            | SpuInstruction::Selb { rt, .. }
            | SpuInstruction::Xsbh { rt, .. }
            | SpuInstruction::Xshw { rt, .. }
            | SpuInstruction::Xswd { rt, .. }
            | SpuInstruction::Clz { rt, .. }
            | SpuInstruction::Cntb { rt, .. }
            | SpuInstruction::Fsmb { rt, .. }
            | SpuInstruction::Fsmh { rt, .. }
            | SpuInstruction::Fsm { rt, .. }
            | SpuInstruction::Gbb { rt, .. }
            | SpuInstruction::Gb { rt, .. }
            | SpuInstruction::Gbh { rt, .. }
            | SpuInstruction::Ori { rt, .. }
            | SpuInstruction::Andc { rt, .. }
            | SpuInstruction::Orc { rt, .. }
            | SpuInstruction::Xor { rt, .. }
            | SpuInstruction::Nand { rt, .. }
            | SpuInstruction::Eqv { rt, .. }
            | SpuInstruction::Orx { rt, .. }
            | SpuInstruction::Andbi { rt, .. }
            | SpuInstruction::Andhi { rt, .. }
            | SpuInstruction::Orbi { rt, .. }
            | SpuInstruction::Orhi { rt, .. }
            | SpuInstruction::Xorbi { rt, .. }
            | SpuInstruction::Xorhi { rt, .. }
            | SpuInstruction::Xori { rt, .. }
            | SpuInstruction::Nor { rt, .. }
            | SpuInstruction::Andi { rt, .. }
            | SpuInstruction::Shufb { rt, .. }
            | SpuInstruction::Shlqbyi { rt, .. }
            | SpuInstruction::Rotqby { rt, .. }
            | SpuInstruction::Rotqbyi { rt, .. }
            | SpuInstruction::Rotqmbyi { rt, .. }
            | SpuInstruction::Shlqbi { rt, .. }
            | SpuInstruction::Shlqbii { rt, .. }
            | SpuInstruction::Rotqbi { rt, .. }
            | SpuInstruction::Rotqbii { rt, .. }
            | SpuInstruction::Rotqmbi { rt, .. }
            | SpuInstruction::Rotqmbii { rt, .. }
            | SpuInstruction::Shlqby { rt, .. }
            | SpuInstruction::Shlqbybi { rt, .. }
            | SpuInstruction::Rotqbybi { rt, .. }
            | SpuInstruction::Rotqmby { rt, .. }
            | SpuInstruction::Rotqmbybi { rt, .. }
            | SpuInstruction::Shl { rt, .. }
            | SpuInstruction::Shli { rt, .. }
            | SpuInstruction::Rotmi { rt, .. }
            | SpuInstruction::Rotmai { rt, .. }
            | SpuInstruction::Rot { rt, .. }
            | SpuInstruction::Roti { rt, .. }
            | SpuInstruction::Rotm { rt, .. }
            | SpuInstruction::Rotma { rt, .. }
            | SpuInstruction::Shlh { rt, .. }
            | SpuInstruction::Shlhi { rt, .. }
            | SpuInstruction::Roth { rt, .. }
            | SpuInstruction::Rothi { rt, .. }
            | SpuInstruction::Rothm { rt, .. }
            | SpuInstruction::Rothmi { rt, .. }
            | SpuInstruction::Rotmah { rt, .. }
            | SpuInstruction::Rotmahi { rt, .. }
            | SpuInstruction::Cbd { rt, .. }
            | SpuInstruction::Cbx { rt, .. }
            | SpuInstruction::Chd { rt, .. }
            | SpuInstruction::Chx { rt, .. }
            | SpuInstruction::Cwd { rt, .. }
            | SpuInstruction::Cwx { rt, .. }
            | SpuInstruction::Cdd { rt, .. }
            | SpuInstruction::Cdx { rt, .. }
            | SpuInstruction::Ceqb { rt, .. }
            | SpuInstruction::Ceqh { rt, .. }
            | SpuInstruction::Ceqhi { rt, .. }
            | SpuInstruction::Cgtb { rt, .. }
            | SpuInstruction::Cgtbi { rt, .. }
            | SpuInstruction::Cgth { rt, .. }
            | SpuInstruction::Cgthi { rt, .. }
            | SpuInstruction::Cgt { rt, .. }
            | SpuInstruction::Clgtb { rt, .. }
            | SpuInstruction::Clgtbi { rt, .. }
            | SpuInstruction::Clgth { rt, .. }
            | SpuInstruction::Clgthi { rt, .. }
            | SpuInstruction::Clgti { rt, .. }
            | SpuInstruction::Ceq { rt, .. }
            | SpuInstruction::Ceqi { rt, .. }
            | SpuInstruction::Ceqbi { rt, .. }
            | SpuInstruction::Cgti { rt, .. }
            | SpuInstruction::Clgt { rt, .. }
            | SpuInstruction::Brsl { rt, .. }
            | SpuInstruction::Brasl { rt, .. }
            | SpuInstruction::Bisl { rt, .. }
            | SpuInstruction::Bisled { rt, .. }
            | SpuInstruction::Rchcnt { rt, .. }
            | SpuInstruction::Mfspr { rt, .. } => Some(rt),
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
                    spu::MFC_CMD => Some(SpuChannelField::CommandQueue),
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
                    // A command that consumed MFC_EAH takes it back to 0.
                    footprint.channels.insert(SpuChannelField::MfcEah);
                    footprint.reservation = true;
                }
                if channel == spu::MFC_WR_TAG_UPDATE {
                    footprint.channels.insert(SpuChannelField::TagUpdate);
                    footprint.channels.insert(SpuChannelField::TagStatusRead);
                }
                None
            }
            SpuInstruction::Br { .. }
            | SpuInstruction::Bra { .. }
            | SpuInstruction::Brz { .. }
            | SpuInstruction::Brnz { .. }
            | SpuInstruction::Bi { .. }
            | SpuInstruction::Brhnz { .. }
            | SpuInstruction::Brhz { .. }
            | SpuInstruction::Biz { .. }
            | SpuInstruction::Binz { .. }
            | SpuInstruction::Bihz { .. }
            | SpuInstruction::Bihnz { .. }
            | SpuInstruction::Iret { .. }
            | SpuInstruction::Nop { .. }
            | SpuInstruction::Lnop
            | SpuInstruction::Hbr { .. }
            | SpuInstruction::Hbra { .. }
            | SpuInstruction::Hbrr { .. }
            | SpuInstruction::Sync { .. }
            | SpuInstruction::Dsync
            | SpuInstruction::Heq { .. }
            | SpuInstruction::Heqi { .. }
            | SpuInstruction::Hgt { .. }
            | SpuInstruction::Hgti { .. }
            | SpuInstruction::Hlgt { .. }
            | SpuInstruction::Hlgti { .. }
            | SpuInstruction::Stop { .. }
            | SpuInstruction::Stopd
            | SpuInstruction::Mtspr { .. }
            | SpuInstruction::Fscrwr { .. } => None,
            SpuInstruction::Fscrrd { rt }
            | SpuInstruction::Fa { rt, .. }
            | SpuInstruction::Fs { rt, .. }
            | SpuInstruction::Fm { rt, .. }
            | SpuInstruction::Fma { rt, .. }
            | SpuInstruction::Fms { rt, .. }
            | SpuInstruction::Fnms { rt, .. }
            | SpuInstruction::Frest { rt, .. }
            | SpuInstruction::Frsqest { rt, .. }
            | SpuInstruction::Fi { rt, .. }
            | SpuInstruction::Csflt { rt, .. }
            | SpuInstruction::Cflts { rt, .. }
            | SpuInstruction::Cuflt { rt, .. }
            | SpuInstruction::Cfltu { rt, .. }
            | SpuInstruction::Fceq { rt, .. }
            | SpuInstruction::Fcmeq { rt, .. }
            | SpuInstruction::Fcgt { rt, .. }
            | SpuInstruction::Fcmgt { rt, .. }
            | SpuInstruction::Dfa { rt, .. }
            | SpuInstruction::Dfs { rt, .. }
            | SpuInstruction::Dfm { rt, .. }
            | SpuInstruction::Dfma { rt, .. }
            | SpuInstruction::Dfms { rt, .. }
            | SpuInstruction::Dfnms { rt, .. }
            | SpuInstruction::Dfnma { rt, .. }
            | SpuInstruction::Frds { rt, .. }
            | SpuInstruction::Fesd { rt, .. } => Some(rt),
        };
        if let Some(register) = register {
            footprint.registers.insert(register);
        }
        footprint.control_transfer = matches!(
            instruction,
            SpuInstruction::Br { .. }
                | SpuInstruction::Brsl { .. }
                | SpuInstruction::Bra { .. }
                | SpuInstruction::Brasl { .. }
                | SpuInstruction::Brz { .. }
                | SpuInstruction::Brnz { .. }
                | SpuInstruction::Bi { .. }
                | SpuInstruction::Bisl { .. }
                | SpuInstruction::Bisled { .. }
                | SpuInstruction::Brhnz { .. }
                | SpuInstruction::Brhz { .. }
                | SpuInstruction::Biz { .. }
                | SpuInstruction::Binz { .. }
                | SpuInstruction::Bihz { .. }
                | SpuInstruction::Bihnz { .. }
                | SpuInstruction::Iret { .. }
        );
        if matches!(
            instruction,
            SpuInstruction::Rdch {
                channel: cellgov_ps3_abi::hw::spu::SPU_RD_IN_MBOX,
                ..
            }
        ) {
            footprint.channels.insert(SpuChannelField::InboundMailbox);
        }
        if matches!(
            instruction,
            SpuInstruction::Rdch {
                channel: cellgov_ps3_abi::hw::spu::MFC_RD_TAG_STAT,
                ..
            }
        ) {
            footprint.channels.insert(SpuChannelField::TagStatusRead);
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
        // [Martignoni2009 p:127 s:2.2] After an exception the program counter, the registers and the memory stay as they were, so a discarded step may change none of them.
        if observed.fault_discarded {
            violations = state_differences(&before, &observed.state);
        } else {
            for (index, (previous, current)) in
                before.regs.iter().zip(&observed.state.regs).enumerate()
            {
                // A stalled access did not retire, so it writes no register.
                // [CBE-Handbook p:542 s:19.6.6.3 SPU Side] An empty inbound mailbox stalls the read.
                if previous != current
                    && (matches!(
                        &observed.outcome,
                        SpuStepOutcome::Yield {
                            reason: cellgov_exec::YieldReason::ChannelStall,
                            ..
                        }
                    ) || !self.registers.contains(&(index as u8)))
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
            if before.fpscr != observed.state.fpscr && !self.fpscr {
                violations.insert(SpuObservationComponent::Fpscr);
            }
            if before.signals != observed.state.signals && !self.signals {
                violations.insert(SpuObservationComponent::Signals);
            }
            if (before.interrupts_enabled, before.srr0)
                != (observed.state.interrupts_enabled, observed.state.srr0)
                && !self.interrupts
            {
                violations.insert(SpuObservationComponent::Interrupts);
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
