//! The SPU reference artifact, its observation and comparison types, and its refusals.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::spu::{MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_ANY};
use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::state::{SpuObservableSnapshot, TagUpdateCondition};
use serde::{Deserialize, Serialize};

use crate::reference::{ReferenceField, ReferenceOmission, ReferenceProvenance};

/// Current offline SPU reference schema.
pub const SPU_REFERENCE_SCHEMA_VERSION: u32 = 5;

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
    /// Initial FPSCR as 32 lowercase hexadecimal digits, bit 0 first;
    /// absent means zero. Only the defined bits may be set.
    ///
    /// [SPU-ISA p:197 s:9.2] RN0 and RN1 select how each double-precision slice rounds, so a vector under a directed mode states them here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpscr: Option<String>,
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
    /// Free MFC command-queue slots, the `MFC_Cmd` count; absent means a
    /// queue with every slot free.
    #[serde(default = "full_command_queue")]
    pub mfc_cmd_count: u32,
    /// The TS code of a waiting tag-status update request.
    ///
    /// The code is 1 for any enabled group and 2 for all of them.
    #[serde(default)]
    pub tag_update: Option<u32>,
    /// The tag status a met update request latched and no read took.
    #[serde(default)]
    pub tag_status_read: Option<u32>,
    /// An atomic command's status is waiting to be read.
    #[serde(default)]
    pub atomic_status_ready: bool,
    /// Messages in the inbound mailbox, oldest first; at most the
    /// mailbox depth.
    #[serde(default)]
    pub in_mbox: Vec<u32>,
    /// The message in the outbound mailbox.
    #[serde(default)]
    pub out_mbox: Option<u32>,
}

fn full_command_queue() -> u32 {
    cellgov_ps3_abi::hw::spu::MFC_SPU_QUEUE_DEPTH
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
            cmd_queue_free,
            tag_update,
            tag_status_read,
            atomic_status_ready,
            in_mbox,
            out_mbox,
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
            mfc_cmd_count: *cmd_queue_free,
            tag_update: tag_update.map(|condition| match condition {
                TagUpdateCondition::Any => MFC_TAG_UPDATE_ANY,
                TagUpdateCondition::All => MFC_TAG_UPDATE_ALL,
            }),
            tag_status_read: *tag_status_read,
            atomic_status_ready: *atomic_status_ready,
            in_mbox: in_mbox.clone(),
            out_mbox: *out_mbox,
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
    /// The instruction stopped the SPU.
    Stop,
}

impl From<&SpuStepOutcome> for SpuReferenceOutcome {
    fn from(outcome: &SpuStepOutcome) -> Self {
        match outcome {
            SpuStepOutcome::Continue => Self::Continue,
            SpuStepOutcome::Branch => Self::Branch,
            SpuStepOutcome::Yield { .. } => Self::Yield,
            SpuStepOutcome::MemoryRead { .. } => Self::MemoryRead,
            SpuStepOutcome::Fault(_) => Self::Fault,
            SpuStepOutcome::Stop { .. } => Self::Stop,
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
    /// Final FPSCR as 32 lowercase hexadecimal digits, bit 0 first. Only
    /// the defined bits may be set.
    pub fpscr: ReferenceField<String>,
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
    /// Floating-point status and control register.
    Fpscr,
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
    /// One vector of a set failed validation or replay.
    #[error("SPU reference vector {index}: {source}")]
    InVector {
        /// The vector's index in its set.
        index: usize,
        /// Why it failed.
        #[source]
        source: Box<SpuReferenceError>,
    },
    /// The scripted world could not service a step.
    #[error("SPU reference world could not service {what}")]
    World {
        /// What it could not service.
        what: &'static str,
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
