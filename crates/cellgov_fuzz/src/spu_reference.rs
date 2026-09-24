//! Offline SPU vectors and operator-supplied hardware observations.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState, SPU_LS_SIZE, SPU_REG_COUNT};
use serde::{Deserialize, Serialize};

use crate::reference::ReferenceField;

/// Current offline SPU reference schema.
pub const SPU_REFERENCE_SCHEMA_VERSION: u32 = 1;

/// Independent source of an SPU observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferenceProvenance {
    /// A vector derived from a cited public SPU instruction rule.
    DocumentedVector {
        /// Official source key, printed page, and section.
        citation: String,
        /// Stable identifier for the chosen source inputs.
        vector_id: String,
    },
    /// An operator-supplied observation from physical hardware.
    HardwareCapture {
        /// Stable operator capture identifier.
        capture_id: String,
        /// Hardware model supplied by the operator.
        device: String,
        /// Firmware and acquisition context supplied by the operator.
        environment: String,
        /// SHA-256 of the original capture artifact.
        source_sha256: String,
    },
}

/// Sparse initial state relative to a fully zeroed SPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceInput {
    /// Register overrides as 32 lowercase hexadecimal digits.
    pub regs_hex: BTreeMap<String, String>,
    /// Local-store byte overrides before instruction words load.
    pub local_store: BTreeMap<String, u8>,
    /// Initial program counter.
    pub pc: u32,
    /// Initial channel state; absent means all channels start at zero.
    pub channels: Option<SpuReferenceChannels>,
    /// Reserved-line address, if present.
    pub reservation: Option<u64>,
}

/// Channel fields represented by the SPU observation contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceChannels {
    /// Staged local-store address.
    pub mfc_lsa: u32,
    /// Staged effective-address high word.
    pub mfc_eah: u32,
    /// Staged effective-address low word.
    pub mfc_eal: u32,
    /// Staged transfer size.
    pub mfc_size: u32,
    /// Staged tag identifier.
    pub mfc_tag_id: u32,
    /// Tag-query mask.
    pub tag_mask: u32,
    /// Completed tag-status word.
    pub tag_status: u32,
    /// Atomic-command status.
    pub atomic_status: u32,
    /// Destination of an unresolved inbound-mailbox read.
    pub pending_mbox_rt: Option<u8>,
    /// Pending DMA GET as `(effective address, local address, size, tag)`.
    pub pending_get: Option<(u64, u32, u32, u8)>,
}

impl From<&cellgov_spu::state::SpuChannelSnapshot> for SpuReferenceChannels {
    fn from(value: &cellgov_spu::state::SpuChannelSnapshot) -> Self {
        let cellgov_spu::state::SpuChannelSnapshot {
            mfc_lsa,
            mfc_eah,
            mfc_eal,
            mfc_size,
            mfc_tag_id,
            tag_mask,
            tag_status,
            atomic_status,
            pending_mbox_rt,
            pending_get,
        } = value;
        Self {
            mfc_lsa: *mfc_lsa,
            mfc_eah: *mfc_eah,
            mfc_eal: *mfc_eal,
            mfc_size: *mfc_size,
            mfc_tag_id: *mfc_tag_id,
            tag_mask: *tag_mask,
            tag_status: *tag_status,
            atomic_status: *atomic_status,
            pending_mbox_rt: *pending_mbox_rt,
            pending_get: *pending_get,
        }
    }
}

/// Coarse terminal result that independent sources can represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceOutcome {
    /// Normal instruction completion.
    Continue,
    /// Explicit control transfer.
    Branch,
    /// Caller-visible yield with possible effects.
    Yield,
    /// Caller-serviced committed-memory read.
    MemoryRead,
    /// Architectural fault.
    Fault,
}

impl From<&SpuStepOutcome> for SpuReferenceOutcome {
    fn from(outcome: &SpuStepOutcome) -> Self {
        match outcome {
            SpuStepOutcome::Continue => Self::Continue,
            SpuStepOutcome::Branch => Self::Branch,
            SpuStepOutcome::Yield { .. } => Self::Yield,
            SpuStepOutcome::MemoryRead { .. } => Self::MemoryRead,
            SpuStepOutcome::Fault(_) => Self::Fault,
        }
    }
}

/// Final observation with explicit coverage on every SPU state axis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceExpected {
    /// Full register bank after sparse overrides of the initial bank.
    pub regs_hex: ReferenceField<BTreeMap<String, String>>,
    /// Full local store after sparse overrides of the loaded initial store.
    pub local_store: ReferenceField<BTreeMap<String, u8>>,
    /// Final program counter.
    pub pc: ReferenceField<u32>,
    /// Complete channel state.
    pub channels: ReferenceField<SpuReferenceChannels>,
    /// Local reserved-line address.
    pub reservation: ReferenceField<Option<u64>>,
    /// Terminal result class.
    pub outcome: ReferenceField<SpuReferenceOutcome>,
    /// Ordered emitted effects; version one represents only an empty list.
    pub effects: ReferenceField<Vec<String>>,
    /// Whether the fault discarded the instruction state.
    pub fault_discarded: ReferenceField<bool>,
}

/// One bounded committed vector or hardware capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceArtifact {
    /// Version of this serialized schema.
    pub schema_version: u32,
    /// Stable fixture identifier.
    pub case_id: String,
    /// Source and acquisition information.
    pub provenance: SpuReferenceProvenance,
    /// Instruction words in local-store program order.
    pub words: Vec<u32>,
    /// Explicit initial state.
    pub initial_state: SpuReferenceInput,
    /// Expected final observation.
    pub expected: SpuReferenceExpected,
}

/// Comparable SPU observation component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuReferenceComponent {
    /// Complete register bank.
    Registers,
    /// Complete local store.
    LocalStore,
    /// Program counter.
    ProgramCounter,
    /// Channel state.
    Channels,
    /// Local reservation.
    Reservation,
    /// Terminal outcome class.
    Outcome,
    /// Emitted effects.
    Effects,
    /// Fault-discard marker.
    FaultDiscard,
}

/// Reason an independent source cannot constrain a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuReferenceOmission {
    /// The architecture leaves the component undefined.
    Undefined,
    /// The acquisition source cannot represent the component.
    Unsupported,
}

/// Result of comparing mutually represented SPU components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuReferenceComparison {
    /// Components with a represented value on both sides.
    pub compared: BTreeSet<SpuReferenceComponent>,
    /// Represented components that disagreed.
    pub differences: BTreeSet<SpuReferenceComponent>,
    /// Components excluded with their source limitation.
    pub unrepresented: BTreeMap<SpuReferenceComponent, SpuReferenceOmission>,
}

impl SpuReferenceComparison {
    /// Tests whether every represented component agreed.
    pub fn is_match(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Offline replay result with its full internal observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuReferenceReplay {
    /// State after the input words have been placed in local store.
    pub initial: SpuObservableSnapshot,
    /// Complete state at the comparison boundary.
    pub state: SpuObservableSnapshot,
    /// Typed terminal outcome, including effect payloads.
    pub outcome: SpuStepOutcome,
    /// Independent comparison result.
    pub comparison: SpuReferenceComparison,
}

/// A malformed or unreplayable reference artifact.
#[derive(Debug, thiserror::Error)]
pub enum SpuReferenceError {
    /// JSON parsing failed.
    #[error("SPU reference JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Unknown schema version.
    #[error("SPU reference schema version {found} is unsupported; expected {supported}")]
    Version {
        /// Artifact version.
        found: u32,
        /// Supported version.
        supported: u32,
    },
    /// A field or provenance value failed validation.
    #[error("SPU reference field {field} is invalid")]
    Invalid {
        /// Invalid field path.
        field: &'static str,
    },
    /// Instruction fetch failed at the selected program counter.
    #[error("SPU reference instruction fetch failed at PC 0x{pc:08x}")]
    Fetch {
        /// Failed program counter.
        pc: u32,
    },
    /// The fetched reference word cannot decode.
    #[error("SPU reference instruction at PC 0x{pc:08x} did not decode: {source}")]
    Decode {
        /// Failed program counter.
        pc: u32,
        /// Exact decoder refusal, including the raw word.
        #[source]
        source: cellgov_spu::instruction::SpuDecodeError,
    },
}

/// Parses and validates a bounded repository reference artifact.
// [Jiang2022 p:4 s:3] Cases come from the machine-readable specification, and a separate engine compares them on real devices and emulators.
pub fn parse_reference_json(json: &str) -> Result<SpuReferenceArtifact, SpuReferenceError> {
    let artifact: SpuReferenceArtifact = serde_json::from_str(json)?;
    artifact.validate()?;
    Ok(artifact)
}

/// Replays a reference without a device, network, or external runner.
// [Martignoni2009 p:127 s:2.3] Both CPUs start from the same synthetic state and execute the case; the comparison reads only their final states.
pub fn replay_reference(
    artifact: &SpuReferenceArtifact,
) -> Result<SpuReferenceReplay, SpuReferenceError> {
    artifact.validate()?;
    let mut initial = artifact.initial_state.to_state()?;
    for (index, word) in artifact.words.iter().enumerate() {
        let offset = artifact.initial_state.pc as usize + index * 4;
        initial.ls[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    let loaded = initial.clone();
    let mut state = initial;
    let mut outcome = SpuStepOutcome::Continue;
    for _ in &artifact.words {
        let pc = state.pc;
        let raw = state.fetch().ok_or(SpuReferenceError::Fetch { pc })?;
        let instruction = cellgov_spu::decode::decode(raw)
            .map_err(|source| SpuReferenceError::Decode { pc, source })?;
        outcome = execute(&instruction, &mut state, UnitId::new(0));
        crate::seeded::spu_observed(
            &instruction,
            cellgov_spu::fuzz::SpuOutcomeClass::from_outcome(&outcome),
            &mut state.regs,
        );
        match outcome {
            SpuStepOutcome::Continue => state.advance_pc(),
            SpuStepOutcome::Branch => {}
            SpuStepOutcome::Yield { .. } | SpuStepOutcome::MemoryRead { .. } => break,
            SpuStepOutcome::Fault(_) => {
                state = loaded.clone();
                break;
            }
        }
    }
    let observed = SpuObservableSnapshot::capture(&state);
    let initial = SpuObservableSnapshot::capture(&loaded);
    let comparison = compare_reference(&artifact.expected, &initial, &observed, &outcome)?;
    Ok(SpuReferenceReplay {
        initial,
        state: observed,
        outcome,
        comparison,
    })
}

/// Compares the independent source against the complete internal observation.
// [Watt2023 p:110:2 s:1] A reference earns its trust from its proven correspondence to the specification, independent of the implementation it checks.
// [Martignoni2009 p:127 s:2.2] The compared state is the program counter, the registers, the memory, and the exception. After an exception the other three stay as they were.
pub fn compare_reference(
    expected: &SpuReferenceExpected,
    loaded: &SpuObservableSnapshot,
    observed: &SpuObservableSnapshot,
    outcome: &SpuStepOutcome,
) -> Result<SpuReferenceComparison, SpuReferenceError> {
    if expected
        .effects
        .as_value()
        .is_some_and(|effects| !effects.is_empty())
    {
        return Err(SpuReferenceError::Invalid {
            field: "expected.effects",
        });
    }
    let mut comparison = SpuReferenceComparison {
        compared: BTreeSet::new(),
        differences: BTreeSet::new(),
        unrepresented: BTreeMap::new(),
    };
    let mut registers = loaded.regs;
    if let ReferenceField::Value { value } = &expected.regs_hex {
        for (index, hex) in value {
            let index = parse_index(index, SPU_REG_COUNT).ok_or(SpuReferenceError::Invalid {
                field: "expected.regs_hex",
            })?;
            registers[index] = parse_register(hex).ok_or(SpuReferenceError::Invalid {
                field: "expected.regs_hex",
            })?;
        }
    }
    compare_field(
        SpuReferenceComponent::Registers,
        &expected.regs_hex,
        registers == observed.regs,
        &mut comparison,
    );
    let mut ls = loaded.ls.clone();
    if let ReferenceField::Value { value } = &expected.local_store {
        for (offset, &byte) in value {
            let offset = parse_index(offset, ls.len()).ok_or(SpuReferenceError::Invalid {
                field: "expected.local_store",
            })?;
            ls[offset] = byte;
        }
    }
    compare_field(
        SpuReferenceComponent::LocalStore,
        &expected.local_store,
        ls == observed.ls,
        &mut comparison,
    );
    compare_field(
        SpuReferenceComponent::ProgramCounter,
        &expected.pc,
        expected
            .pc
            .as_value()
            .is_none_or(|value| *value == observed.pc),
        &mut comparison,
    );
    compare_field(
        SpuReferenceComponent::Channels,
        &expected.channels,
        expected
            .channels
            .as_value()
            .is_none_or(|value| *value == SpuReferenceChannels::from(&observed.channels)),
        &mut comparison,
    );
    compare_field(
        SpuReferenceComponent::Reservation,
        &expected.reservation,
        expected
            .reservation
            .as_value()
            .is_none_or(|value| *value == observed.reservation.map(|line| line.addr())),
        &mut comparison,
    );
    compare_field(
        SpuReferenceComponent::Outcome,
        &expected.outcome,
        expected
            .outcome
            .as_value()
            .is_none_or(|value| *value == SpuReferenceOutcome::from(outcome)),
        &mut comparison,
    );
    let effects_empty = match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects.is_empty(),
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_) => true,
    };
    compare_field(
        SpuReferenceComponent::Effects,
        &expected.effects,
        effects_empty,
        &mut comparison,
    );
    compare_field(
        SpuReferenceComponent::FaultDiscard,
        &expected.fault_discarded,
        expected
            .fault_discarded
            .as_value()
            .is_none_or(|value| *value == matches!(outcome, SpuStepOutcome::Fault(_))),
        &mut comparison,
    );
    Ok(comparison)
}

// [Jiang2022 p:7 s:4.2] Most device and emulator inconsistencies trace to behaviour the manual leaves undefined, so a component the documentation marks undefined is excluded rather than counted as a difference.
fn compare_field<T>(
    component: SpuReferenceComponent,
    field: &ReferenceField<T>,
    agrees: bool,
    comparison: &mut SpuReferenceComparison,
) {
    match field {
        ReferenceField::Value { .. } => {
            comparison.compared.insert(component);
            if !agrees {
                comparison.differences.insert(component);
            }
        }
        ReferenceField::Undefined { .. } => {
            comparison
                .unrepresented
                .insert(component, SpuReferenceOmission::Undefined);
        }
        ReferenceField::Unsupported { .. } => {
            comparison
                .unrepresented
                .insert(component, SpuReferenceOmission::Unsupported);
        }
    }
}

impl<T> ReferenceField<T> {
    fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value { value } => Some(value),
            Self::Undefined { .. } | Self::Unsupported { .. } => None,
        }
    }
}

fn parse_register(hex: &str) -> Option<[u8; 16]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 32
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return None;
    }
    let mut result = [0u8; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        let high = (bytes[index * 2] as char).to_digit(16)? as u8;
        let low = (bytes[index * 2 + 1] as char).to_digit(16)? as u8;
        *byte = high << 4 | low;
    }
    Some(result)
}

fn parse_index(key: &str, limit: usize) -> Option<usize> {
    let index = key.parse::<usize>().ok()?;
    (index < limit && index.to_string() == key).then_some(index)
}

impl SpuReferenceInput {
    fn to_state(&self) -> Result<SpuState, SpuReferenceError> {
        let mut state = SpuState::new();
        state.pc = self.pc;
        for (index, hex) in &self.regs_hex {
            let index = parse_index(index, SPU_REG_COUNT).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.regs_hex",
            })?;
            state.regs[index] = parse_register(hex).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.regs_hex",
            })?;
        }
        for (offset, &byte) in &self.local_store {
            let offset = parse_index(offset, SPU_LS_SIZE).ok_or(SpuReferenceError::Invalid {
                field: "initial_state.local_store",
            })?;
            state.ls[offset] = byte;
        }
        if let Some(channels) = &self.channels {
            state.channels.mfc_lsa = channels.mfc_lsa;
            state.channels.mfc_eah = channels.mfc_eah;
            state.channels.mfc_eal = channels.mfc_eal;
            state.channels.mfc_size = channels.mfc_size;
            state.channels.mfc_tag_id = channels.mfc_tag_id;
            state.channels.tag_mask = channels.tag_mask;
            state.channels.tag_status = channels.tag_status;
            state.channels.atomic_status = channels.atomic_status;
            state.channels.pending_mbox_rt = channels.pending_mbox_rt;
            state.channels.pending_get = channels.pending_get;
        }
        state.reservation = self.reservation.map(cellgov_sync::ReservedLine::containing);
        Ok(state)
    }
}

impl SpuReferenceArtifact {
    fn validate(&self) -> Result<(), SpuReferenceError> {
        let invalid = |field| SpuReferenceError::Invalid { field };
        if self.schema_version != SPU_REFERENCE_SCHEMA_VERSION {
            return Err(SpuReferenceError::Version {
                found: self.schema_version,
                supported: SPU_REFERENCE_SCHEMA_VERSION,
            });
        }
        if self.case_id.trim().is_empty() {
            return Err(invalid("case_id"));
        }
        if self.words.is_empty()
            || self.words.len() > 64
            || self.initial_state.pc & 3 != 0
            || (self.initial_state.pc as usize)
                .checked_add(self.words.len() * 4)
                .is_none_or(|end| end > SPU_LS_SIZE)
        {
            return Err(invalid("words"));
        }
        for (field, regs) in [
            ("initial_state.regs_hex", Some(&self.initial_state.regs_hex)),
            ("expected.regs_hex", self.expected.regs_hex.as_value()),
        ] {
            if regs.is_some_and(|regs| {
                regs.iter().any(|(index, hex)| {
                    parse_index(index, SPU_REG_COUNT).is_none() || parse_register(hex).is_none()
                })
            }) {
                return Err(invalid(field));
            }
        }
        if self
            .initial_state
            .local_store
            .keys()
            .any(|offset| parse_index(offset, SPU_LS_SIZE).is_none())
        {
            return Err(invalid("initial_state.local_store"));
        }
        if self.expected.local_store.as_value().is_some_and(|ls| {
            ls.keys()
                .any(|offset| parse_index(offset, SPU_LS_SIZE).is_none())
        }) {
            return Err(invalid("expected.local_store"));
        }
        if self
            .initial_state
            .reservation
            .is_some_and(|addr| cellgov_sync::ReservedLine::containing(addr).addr() != addr)
        {
            return Err(invalid("initial_state.reservation"));
        }
        if self
            .expected
            .reservation
            .as_value()
            .is_some_and(|reservation| {
                reservation
                    .is_some_and(|addr| cellgov_sync::ReservedLine::containing(addr).addr() != addr)
            })
        {
            return Err(invalid("expected.reservation"));
        }
        if self
            .initial_state
            .channels
            .as_ref()
            .is_some_and(|channels| {
                channels
                    .pending_mbox_rt
                    .is_some_and(|register| register as usize >= SPU_REG_COUNT)
            })
        {
            return Err(invalid("initial_state.channels.pending_mbox_rt"));
        }
        if self.expected.channels.as_value().is_some_and(|channels| {
            channels
                .pending_mbox_rt
                .is_some_and(|register| register as usize >= SPU_REG_COUNT)
        }) {
            return Err(invalid("expected.channels.pending_mbox_rt"));
        }
        if self
            .expected
            .effects
            .as_value()
            .is_some_and(|effects| !effects.is_empty())
        {
            return Err(invalid("expected.effects"));
        }
        for (field, reason) in [
            (
                "expected.regs_hex",
                omission_reason(&self.expected.regs_hex),
            ),
            (
                "expected.local_store",
                omission_reason(&self.expected.local_store),
            ),
            ("expected.pc", omission_reason(&self.expected.pc)),
            (
                "expected.channels",
                omission_reason(&self.expected.channels),
            ),
            (
                "expected.reservation",
                omission_reason(&self.expected.reservation),
            ),
            ("expected.outcome", omission_reason(&self.expected.outcome)),
            ("expected.effects", omission_reason(&self.expected.effects)),
            (
                "expected.fault_discarded",
                omission_reason(&self.expected.fault_discarded),
            ),
        ] {
            if reason.is_some_and(|reason| reason.trim().is_empty()) {
                return Err(invalid(field));
            }
        }
        match &self.provenance {
            SpuReferenceProvenance::DocumentedVector {
                citation,
                vector_id,
            } => {
                if !valid_spu_citation(citation) || vector_id.trim().is_empty() {
                    return Err(invalid("provenance"));
                }
            }
            SpuReferenceProvenance::HardwareCapture {
                capture_id,
                device,
                environment,
                source_sha256,
            } => {
                if [capture_id, device, environment]
                    .iter()
                    .any(|value| value.trim().is_empty())
                    || source_sha256.len() != 64
                    || !source_sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(invalid("provenance"));
                }
            }
        }
        Ok(())
    }
}

fn valid_spu_citation(citation: &str) -> bool {
    let Some((page, section)) = citation
        .strip_prefix("SPU-ISA p:")
        .and_then(|remainder| remainder.split_once(" s:"))
    else {
        return false;
    };
    let Ok(number) = page.parse::<u16>() else {
        return false;
    };
    number > 0
        && number.to_string() == page
        && !section.trim().is_empty()
        && section.trim() == section
}

fn omission_reason<T>(field: &ReferenceField<T>) -> Option<&str> {
    match field {
        ReferenceField::Value { .. } => None,
        ReferenceField::Undefined { reason } | ReferenceField::Unsupported { reason } => {
            Some(reason)
        }
    }
}

#[cfg(test)]
#[path = "tests/spu_reference_tests.rs"]
mod tests;
