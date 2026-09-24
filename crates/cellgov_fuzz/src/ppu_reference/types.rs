//! The PPU reference artifact, its observation and comparison types, and its refusals.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ppu_paths::{PpuPathDivergence, PpuPathRun};
use crate::reference::{ReferenceField, ReferenceOmission, ReferenceProvenance};

/// Current repository-data schema version.
pub const PPU_REFERENCE_SCHEMA_VERSION: u32 = 1;

/// Provenance for an independent PPU observation.
pub type PpuReferenceProvenance = ReferenceProvenance;

/// Initial-state construction for a reference case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceInputState {
    /// Must be `zeroed`; sparse overrides apply to that named base.
    pub base: PpuReferenceStateBase,
    /// Sparse general-purpose register overrides.
    pub gpr: BTreeMap<u8, u64>,
    /// Sparse floating-point register overrides as raw bits.
    pub fpr: BTreeMap<u8, u64>,
    /// Sparse vector register overrides as 32-digit hexadecimal strings.
    pub vr_hex: BTreeMap<u8, String>,
    /// Initial program counter.
    pub pc: u64,
    /// Initial condition register.
    pub cr: u32,
    /// Initial link register.
    pub lr: u64,
    /// Initial count register.
    pub ctr: u64,
    /// Initial fixed-point exception register.
    pub xer: u64,
    /// Initial AltiVec usage mask.
    pub vrsave: u32,
    /// Initial time-base value.
    pub tb: u64,
    /// Initial reserved-line address.
    pub reservation: Option<u64>,
}

/// Named base state for sparse initial-state construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PpuReferenceStateBase {
    /// Every PPU state field starts at zero with no reservation.
    Zeroed,
}

/// Expected architectural state with explicit field coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceState {
    /// General-purpose register bank.
    pub gpr: ReferenceField<Vec<u64>>,
    /// Floating-point register bank as raw bits.
    pub fpr: ReferenceField<Vec<u64>>,
    /// Vector register bank as 32-digit hexadecimal strings.
    pub vr_hex: ReferenceField<Vec<String>>,
    /// Program counter.
    pub pc: ReferenceField<u64>,
    /// Condition register.
    pub cr: ReferenceField<u32>,
    /// Link register.
    pub lr: ReferenceField<u64>,
    /// Count register.
    pub ctr: ReferenceField<u64>,
    /// Fixed-point exception register.
    pub xer: ReferenceField<u64>,
    /// AltiVec usage mask.
    pub vrsave: ReferenceField<u32>,
    /// Time-base value.
    pub tb: ReferenceField<u64>,
    /// Reserved-line address.
    pub reservation: ReferenceField<Option<u64>>,
}

/// Normalized stop attribution in a reference artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceStop {
    /// Yield reason.
    pub reason: ReferenceField<PpuReferenceYieldReason>,
    /// Fault attribution.
    pub fault: ReferenceField<PpuReferenceFault>,
    /// Attributed program counter.
    pub pc: ReferenceField<Option<u64>>,
    /// Caller return address for a syscall.
    pub lr: ReferenceField<Option<u64>>,
    /// System-call LEV field.
    pub syscall_lev: ReferenceField<Option<u8>>,
    /// Effective address of a faulting memory access.
    pub faulting_ea: ReferenceField<Option<u64>>,
    /// Register snapshot at the fault site.
    pub fault_registers: ReferenceField<Option<PpuReferenceFaultRegisters>>,
    /// Raw syscall number and arguments.
    pub syscall_args: ReferenceField<Option<Vec<u64>>>,
}

/// Fault-site registers in an authoritative observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceFaultRegisters {
    /// General-purpose registers.
    pub gpr: Vec<u64>,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// Condition register.
    pub cr: u32,
}

/// Stable serialized names for runtime yield reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PpuReferenceYieldReason {
    /// The instruction budget ended.
    BudgetExhausted,
    /// Execution reached a syscall.
    Syscall,
    /// Execution needs mailbox arbitration.
    MailboxAccess,
    /// Execution submitted a DMA request.
    DmaSubmitted,
    /// Execution waits for DMA completion.
    DmaWait,
    /// Execution waits on a synchronization primitive.
    WaitingSync,
    /// Execution faulted.
    Fault,
    /// Execution reached an interrupt boundary.
    InterruptBoundary,
    /// Execution finished.
    Finished,
}

/// Stable serialized fault attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PpuReferenceFault {
    /// The step did not fault.
    None,
    /// The commit pipeline rejected an effect.
    Validation,
    /// The guest raised an architecture-defined fault code.
    Guest {
        /// Guest fault code.
        code: u32,
    },
}

/// Complete normalized final observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceObservation {
    /// Architectural state.
    pub state: PpuReferenceState,
    /// Observed data bytes.
    pub memory: ReferenceField<Vec<u8>>,
    /// Terminal attribution.
    pub stop: PpuReferenceStop,
    /// Number of retired instruction slots.
    pub retired: ReferenceField<u64>,
    /// Effects before commit, in emission order.
    pub staged_effects: ReferenceField<Vec<String>>,
    /// Effects accepted by commit, in emission order.
    pub committed_effects: ReferenceField<Vec<String>>,
    /// Runtime reservation table as `(unit, line)` pairs.
    pub reservations: ReferenceField<Vec<(u64, u64)>>,
    /// Pending stores at the comparison boundary.
    pub store_buffer: ReferenceField<Vec<String>>,
    /// Commit refusal rendered as a stable diagnostic value.
    pub commit_error: ReferenceField<Option<String>>,
    /// Whether a fault discarded the batch.
    pub fault_discarded: ReferenceField<bool>,
}

/// One committed PPU reference artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceArtifact {
    /// Schema version.
    pub schema_version: u32,
    /// Stable case identifier.
    pub case_id: String,
    /// Independent source of the expected observation.
    pub provenance: PpuReferenceProvenance,
    /// Instruction words in execution order.
    pub words: Vec<u32>,
    /// Initial PPU state.
    pub initial_state: PpuReferenceInputState,
    /// Base address for `initial_memory`.
    pub memory_base: u64,
    /// Initial data bytes.
    pub initial_memory: Vec<u8>,
    /// Expected final observation.
    pub expected: PpuReferenceObservation,
}

/// One represented field that differs from the reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuReferenceDifference {
    /// Stable field path.
    pub field: PpuReferenceComponent,
    /// Expected value.
    pub expected: String,
    /// Observed value.
    pub observed: String,
}

/// One field excluded for an explicit source limitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuUnrepresentedField {
    /// Stable field path.
    pub field: PpuReferenceComponent,
    /// Why the field cannot participate.
    pub status: PpuReferenceFieldStatus,
    /// Source-provided reason.
    pub reason: String,
}

/// Explicit non-value status for a reference field.
pub type PpuReferenceFieldStatus = ReferenceOmission;

/// Typed component of a normalized PPU reference observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuReferenceComponent {
    /// General-purpose registers.
    StateGpr,
    /// Floating-point registers.
    StateFpr,
    /// Vector registers.
    StateVr,
    /// Program counter.
    StatePc,
    /// Condition register.
    StateCr,
    /// Link register.
    StateLr,
    /// Count register.
    StateCtr,
    /// Fixed-point exception register.
    StateXer,
    /// AltiVec usage mask.
    StateVrsave,
    /// Time-base value.
    StateTb,
    /// Local reservation.
    StateReservation,
    /// Observed memory.
    Memory,
    /// Yield reason.
    StopReason,
    /// Fault attribution.
    StopFault,
    /// Stop program counter.
    StopPc,
    /// Stop link register.
    StopLr,
    /// System-call LEV field.
    StopSyscallLev,
    /// Faulting effective address.
    StopFaultingEa,
    /// Fault-site registers.
    StopFaultRegisters,
    /// System-call arguments.
    StopSyscallArgs,
    /// Retired instruction count.
    Retired,
    /// Staged effect sequence.
    StagedEffects,
    /// Committed effect sequence.
    CommittedEffects,
    /// Runtime reservations.
    Reservations,
    /// Pending store buffer.
    StoreBuffer,
    /// Commit refusal.
    CommitError,
    /// Fault-discard marker.
    FaultDiscarded,
}

/// Comparison against the mutually represented observation fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuReferenceComparison {
    /// Fields that participated in the comparison.
    pub compared: BTreeSet<PpuReferenceComponent>,
    /// Typed disagreements in field order.
    pub differences: Vec<PpuReferenceDifference>,
    /// Fields excluded for an explicit reason.
    pub unrepresented: Vec<PpuUnrepresentedField>,
}

impl PpuReferenceComparison {
    /// Reports whether all mutually represented fields agree.
    pub fn is_match(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Offline replay result for every internal execution path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuReferenceReplay {
    /// Internal path runs.
    pub runs: Vec<PpuPathRun>,
    /// First internal disagreement, if any.
    pub internal_divergence: Option<PpuPathDivergence>,
    /// Independent-reference comparison for each path.
    pub comparisons: Vec<PpuReferenceComparison>,
}

/// Failure to parse, validate, or execute a reference artifact.
#[derive(Debug, thiserror::Error)]
pub enum PpuReferenceError {
    /// JSON parsing failed.
    #[error("PPU reference JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Schema version is not supported.
    #[error("PPU reference schema version {found} is unsupported; expected {supported}")]
    Version {
        /// Artifact version.
        found: u32,
        /// Supported version.
        supported: u32,
    },
    /// A register-bank value has the wrong element count.
    #[error("PPU reference field {field} has {found} entries; expected {expected}")]
    FieldLength {
        /// Field path.
        field: &'static str,
        /// Observed count.
        found: usize,
        /// Required count.
        expected: usize,
    },
    /// A sparse register index is outside its register bank.
    #[error("PPU reference {bank} index {index} is outside 0..32")]
    RegisterIndex {
        /// Register bank name.
        bank: &'static str,
        /// Rejected index.
        index: u8,
    },
    /// A vector-register value is not a fixed-width hexadecimal value.
    #[error("PPU reference vector register {index} must contain 32 lowercase hexadecimal digits")]
    VectorValue {
        /// Register index.
        index: u8,
    },
    /// A reserved-line address is not 128-byte aligned.
    #[error("PPU reference reservation 0x{address:016x} is not 128-byte aligned")]
    ReservationAlignment {
        /// Rejected address.
        address: u64,
    },
    /// The data base is outside the execution-path contract.
    #[error("PPU reference data base 0x{found:016x} is unsupported; expected 0x{expected:016x}")]
    MemoryBase {
        /// Artifact base.
        found: u64,
        /// Supported base.
        expected: u64,
    },
    /// Hardware provenance has an invalid digest.
    #[error("PPU hardware capture SHA-256 must contain 64 lowercase hexadecimal digits")]
    CaptureDigest,
    /// Documented-vector provenance does not name an official citation.
    #[error(
        "PPU documented vector citation is not a supported official-source citation: {citation}"
    )]
    Citation {
        /// Rejected citation.
        citation: String,
    },
    /// An omission reason contains no text.
    #[error("PPU reference field {field} has an empty {status} reason")]
    EmptyReason {
        /// Field path.
        field: &'static str,
        /// Omission status.
        status: &'static str,
    },
    /// A required provenance field contains no text.
    #[error("PPU reference provenance field {field} must not be empty")]
    EmptyProvenance {
        /// Field name.
        field: &'static str,
    },
    /// Schema version one cannot normalize a represented value for this field.
    #[error("PPU reference schema version one cannot represent non-empty field {field}; mark it unsupported")]
    UnsupportedValue {
        /// Field path.
        field: &'static str,
    },
    /// The execution-path harness refused the case.
    #[error("PPU reference replay failed: {0}")]
    Replay(#[from] crate::ppu_paths::PpuPathError),
}
