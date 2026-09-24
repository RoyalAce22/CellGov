//! The SPU reference artifact, its observation and comparison types, and its refusals.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::state::SpuObservableSnapshot;
use serde::{Deserialize, Serialize};

use crate::reference::{ReferenceField, ReferenceOmission, ReferenceProvenance};

/// Current offline SPU reference schema.
pub const SPU_REFERENCE_SCHEMA_VERSION: u32 = 1;

/// Independent source of an SPU observation.
pub type SpuReferenceProvenance = ReferenceProvenance;

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
pub type SpuReferenceOmission = ReferenceOmission;

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
