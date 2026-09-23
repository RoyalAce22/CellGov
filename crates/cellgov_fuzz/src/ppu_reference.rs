//! Versioned offline references for PPU execution observations.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_effects::Effect;
use cellgov_exec::YieldReason;
use cellgov_ppu::observation::PpuArchitecturalState;
use cellgov_ppu::state::PpuState;
use serde::{Deserialize, Serialize};

use crate::ppu_paths::{first_path_divergence, run_all_paths, PpuPathDivergence, PpuPathRun};

/// Current repository-data schema version.
pub const PPU_REFERENCE_SCHEMA_VERSION: u32 = 1;
const REFERENCE_DATA_BASE: u64 = 0x1000_0000;

/// A represented value or an explicit reason that no comparison is valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceField<T> {
    /// The source represents this field.
    Value {
        /// Expected value.
        value: T,
    },
    /// The source states that the field is architecturally undefined.
    Undefined {
        /// Source-specific reason.
        reason: String,
    },
    /// The source or capture format cannot represent the field.
    Unsupported {
        /// Source-specific reason.
        reason: String,
    },
}

/// Provenance for an independent PPU observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PpuReferenceProvenance {
    /// A vector transcribed from public architecture documentation.
    DocumentedVector {
        /// Repository citation key, page, and section.
        citation: String,
        /// Stable vector identifier within the cited source.
        vector_id: String,
    },
    /// A capture acquired on physical hardware by an operator.
    HardwareCapture {
        /// Stable capture identifier.
        capture_id: String,
        /// Hardware model reported by the operator.
        device: String,
        /// Software or firmware context reported by the operator.
        environment: String,
        /// SHA-256 digest of the source capture artifact.
        source_sha256: String,
    },
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuReferenceFieldStatus {
    /// Architecture documentation marks the field undefined.
    Undefined,
    /// The source cannot represent the field.
    Unsupported,
}

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

/// Parses and validates a repository-data artifact.
// [Jiang2022 p:1 s:Abstract] Representative streams and their source use a machine-readable form.
pub fn parse_reference_json(json: &str) -> Result<PpuReferenceArtifact, PpuReferenceError> {
    let artifact: PpuReferenceArtifact = serde_json::from_str(json)?;
    artifact.validate()?;
    Ok(artifact)
}

/// Replays one artifact without hardware, a network, or an external executable.
// [Martignoni2009 p:129 s:Abstract] The same case runs on the implementation and physical CPU before final-state comparison.
pub fn replay_reference(
    artifact: &PpuReferenceArtifact,
) -> Result<PpuReferenceReplay, PpuReferenceError> {
    artifact.validate()?;
    let initial = artifact.initial_state.to_state()?;
    let runs = run_all_paths(&artifact.words, &initial, &artifact.initial_memory)?;
    let internal_divergence = first_path_divergence(&runs);
    let comparisons = runs
        .iter()
        .map(|run| compare_reference(&artifact.expected, run))
        .collect();
    Ok(PpuReferenceReplay {
        runs,
        internal_divergence,
        comparisons,
    })
}

/// Compares only fields represented by both the reference and CellGov.
// [Watt2023 p:110:1 s:Abstract] A practical oracle retains a justified relationship to its specification.
pub fn compare_reference(
    expected: &PpuReferenceObservation,
    run: &PpuPathRun,
) -> PpuReferenceComparison {
    let mut comparison = PpuReferenceComparison {
        compared: BTreeSet::new(),
        differences: Vec::new(),
        unrepresented: Vec::new(),
    };
    compare_state(&expected.state, &run.observation.state, &mut comparison);
    compare_field(
        PpuReferenceComponent::Memory,
        &expected.memory,
        &run.observation.memory,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopReason,
        &expected.stop.reason,
        &reference_yield(run.stop.reason),
        &mut comparison,
    );
    let fault = match run.stop.fault.as_ref() {
        None => PpuReferenceFault::None,
        Some(cellgov_effects::FaultKind::Validation) => PpuReferenceFault::Validation,
        Some(cellgov_effects::FaultKind::Guest(code)) => PpuReferenceFault::Guest { code: *code },
    };
    compare_field(
        PpuReferenceComponent::StopFault,
        &expected.stop.fault,
        &fault,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopPc,
        &expected.stop.pc,
        &run.stop.pc,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopLr,
        &expected.stop.lr,
        &run.stop.diagnostics.lr,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopSyscallLev,
        &expected.stop.syscall_lev,
        &run.stop.diagnostics.syscall_lev,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopFaultingEa,
        &expected.stop.faulting_ea,
        &run.stop.diagnostics.faulting_ea,
        &mut comparison,
    );
    let fault_registers =
        run.stop
            .diagnostics
            .fault_regs
            .as_ref()
            .map(|registers| PpuReferenceFaultRegisters {
                gpr: registers.gprs.to_vec(),
                lr: registers.lr,
                ctr: registers.ctr,
                xer: registers.xer,
                cr: registers.cr,
            });
    compare_field(
        PpuReferenceComponent::StopFaultRegisters,
        &expected.stop.fault_registers,
        &fault_registers,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StopSyscallArgs,
        &expected.stop.syscall_args,
        &run.stop.syscall_args.map(|args| args.to_vec()),
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::Retired,
        &expected.retired,
        &run.retired,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::StagedEffects,
        &expected.staged_effects,
        &render_effects(&run.observation.staged_effects),
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::CommittedEffects,
        &expected.committed_effects,
        &render_effects(&run.observation.committed_effects),
        &mut comparison,
    );
    let reservations = run
        .observation
        .reservations
        .iter()
        .map(|(unit, line)| (unit.raw(), line.addr()))
        .collect::<Vec<_>>();
    compare_field(
        PpuReferenceComponent::Reservations,
        &expected.reservations,
        &reservations,
        &mut comparison,
    );
    let stores = run
        .observation
        .store_buffer
        .iter()
        .map(|store| format!("{store:?}"))
        .collect::<Vec<_>>();
    compare_field(
        PpuReferenceComponent::StoreBuffer,
        &expected.store_buffer,
        &stores,
        &mut comparison,
    );
    let commit_error = run
        .observation
        .commit_error
        .as_ref()
        .map(ToString::to_string);
    compare_field(
        PpuReferenceComponent::CommitError,
        &expected.commit_error,
        &commit_error,
        &mut comparison,
    );
    compare_field(
        PpuReferenceComponent::FaultDiscarded,
        &expected.fault_discarded,
        &run.observation.fault_discarded,
        &mut comparison,
    );
    comparison
}

impl PpuReferenceArtifact {
    fn validate(&self) -> Result<(), PpuReferenceError> {
        if self.schema_version != PPU_REFERENCE_SCHEMA_VERSION {
            return Err(PpuReferenceError::Version {
                found: self.schema_version,
                supported: PPU_REFERENCE_SCHEMA_VERSION,
            });
        }
        if self.memory_base != REFERENCE_DATA_BASE {
            return Err(PpuReferenceError::MemoryBase {
                found: self.memory_base,
                expected: REFERENCE_DATA_BASE,
            });
        }
        validate_indices("GPR", self.initial_state.gpr.keys().copied())?;
        validate_indices("FPR", self.initial_state.fpr.keys().copied())?;
        validate_indices("VR", self.initial_state.vr_hex.keys().copied())?;
        for (&index, value) in &self.initial_state.vr_hex {
            validate_vector_hex(index, value)?;
        }
        if let Some(address) = self.initial_state.reservation {
            if cellgov_sync::ReservedLine::containing(address).addr() != address {
                return Err(PpuReferenceError::ReservationAlignment { address });
            }
        }
        validate_bank("state.gpr", &self.expected.state.gpr, 32)?;
        validate_bank("state.fpr", &self.expected.state.fpr, 32)?;
        validate_bank("state.vr_hex", &self.expected.state.vr_hex, 32)?;
        if let ReferenceField::Value { value } = &self.expected.state.vr_hex {
            for (index, value) in value.iter().enumerate() {
                validate_vector_hex(index as u8, value)?;
            }
        }
        if let ReferenceField::Value {
            value: Some(registers),
        } = &self.expected.stop.fault_registers
        {
            if registers.gpr.len() != 32 {
                return Err(PpuReferenceError::FieldLength {
                    field: "stop.fault_registers.gpr",
                    found: registers.gpr.len(),
                    expected: 32,
                });
            }
        }
        if let ReferenceField::Value { value: Some(args) } = &self.expected.stop.syscall_args {
            if args.len() != 9 {
                return Err(PpuReferenceError::FieldLength {
                    field: "stop.syscall_args",
                    found: args.len(),
                    expected: 9,
                });
            }
        }
        validate_empty_only("staged_effects", &self.expected.staged_effects)?;
        validate_empty_only("committed_effects", &self.expected.committed_effects)?;
        validate_empty_only("store_buffer", &self.expected.store_buffer)?;
        if let ReferenceField::Value { value: Some(_) } = &self.expected.commit_error {
            return Err(PpuReferenceError::UnsupportedValue {
                field: "commit_error",
            });
        }
        validate_observation_reasons(&self.expected)?;
        match &self.provenance {
            PpuReferenceProvenance::DocumentedVector {
                citation,
                vector_id,
            } => {
                validate_provenance_text("vector_id", vector_id)?;
                let supported_key = ["PPC-Book1", "PPC-Book2", "PPC-Book3", "AltiVec-PEM"]
                    .iter()
                    .any(|key| citation.starts_with(key));
                if !supported_key || !citation.contains(" p:") || !citation.contains(" s:") {
                    return Err(PpuReferenceError::Citation {
                        citation: citation.clone(),
                    });
                }
            }
            PpuReferenceProvenance::HardwareCapture {
                capture_id,
                device,
                environment,
                source_sha256,
            } => {
                validate_provenance_text("capture_id", capture_id)?;
                validate_provenance_text("device", device)?;
                validate_provenance_text("environment", environment)?;
                if source_sha256.len() != 64
                    || !source_sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(PpuReferenceError::CaptureDigest);
                }
            }
        }
        Ok(())
    }
}

impl PpuReferenceInputState {
    fn to_state(&self) -> Result<PpuState, PpuReferenceError> {
        let mut state = match self.base {
            PpuReferenceStateBase::Zeroed => PpuState::new(),
        };
        for (&index, &value) in &self.gpr {
            if index >= 32 {
                return Err(PpuReferenceError::RegisterIndex { bank: "GPR", index });
            }
            state.set_gpr(usize::from(index), value);
        }
        for (&index, &value) in &self.fpr {
            state.set_fpr(usize::from(index), value);
        }
        for (&index, value) in &self.vr_hex {
            let parsed = u128::from_str_radix(value, 16)
                .map_err(|_| PpuReferenceError::VectorValue { index })?;
            state.set_vr(usize::from(index), parsed);
        }
        state.pc = self.pc;
        state.set_cr(self.cr);
        state.set_lr(self.lr);
        state.set_ctr(self.ctr);
        state.set_xer(self.xer);
        state.vrsave = self.vrsave;
        state.tb = self.tb;
        state.set_reservation(self.reservation.map(cellgov_sync::ReservedLine::containing));
        Ok(state)
    }
}

fn validate_bank<T>(
    field: &'static str,
    value: &ReferenceField<Vec<T>>,
    expected: usize,
) -> Result<(), PpuReferenceError> {
    if let ReferenceField::Value { value } = value {
        if value.len() != expected {
            return Err(PpuReferenceError::FieldLength {
                field,
                found: value.len(),
                expected,
            });
        }
    }
    Ok(())
}

fn validate_empty_only<T>(
    field: &'static str,
    value: &ReferenceField<Vec<T>>,
) -> Result<(), PpuReferenceError> {
    if let ReferenceField::Value { value } = value {
        if !value.is_empty() {
            return Err(PpuReferenceError::UnsupportedValue { field });
        }
    }
    Ok(())
}

fn validate_indices(
    bank: &'static str,
    indices: impl Iterator<Item = u8>,
) -> Result<(), PpuReferenceError> {
    for index in indices {
        if index >= 32 {
            return Err(PpuReferenceError::RegisterIndex { bank, index });
        }
    }
    Ok(())
}

fn validate_vector_hex(index: u8, value: &str) -> Result<(), PpuReferenceError> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PpuReferenceError::VectorValue { index });
    }
    Ok(())
}

fn validate_observation_reasons(
    observation: &PpuReferenceObservation,
) -> Result<(), PpuReferenceError> {
    let state = &observation.state;
    validate_reason("state.gpr", &state.gpr)?;
    validate_reason("state.fpr", &state.fpr)?;
    validate_reason("state.vr_hex", &state.vr_hex)?;
    validate_reason("state.pc", &state.pc)?;
    validate_reason("state.cr", &state.cr)?;
    validate_reason("state.lr", &state.lr)?;
    validate_reason("state.ctr", &state.ctr)?;
    validate_reason("state.xer", &state.xer)?;
    validate_reason("state.vrsave", &state.vrsave)?;
    validate_reason("state.tb", &state.tb)?;
    validate_reason("state.reservation", &state.reservation)?;
    validate_reason("memory", &observation.memory)?;
    validate_reason("stop.reason", &observation.stop.reason)?;
    validate_reason("stop.fault", &observation.stop.fault)?;
    validate_reason("stop.pc", &observation.stop.pc)?;
    validate_reason("stop.lr", &observation.stop.lr)?;
    validate_reason("stop.syscall_lev", &observation.stop.syscall_lev)?;
    validate_reason("stop.faulting_ea", &observation.stop.faulting_ea)?;
    validate_reason("stop.fault_registers", &observation.stop.fault_registers)?;
    validate_reason("stop.syscall_args", &observation.stop.syscall_args)?;
    validate_reason("retired", &observation.retired)?;
    validate_reason("staged_effects", &observation.staged_effects)?;
    validate_reason("committed_effects", &observation.committed_effects)?;
    validate_reason("reservations", &observation.reservations)?;
    validate_reason("store_buffer", &observation.store_buffer)?;
    validate_reason("commit_error", &observation.commit_error)?;
    validate_reason("fault_discarded", &observation.fault_discarded)
}

fn validate_reason<T>(
    field: &'static str,
    value: &ReferenceField<T>,
) -> Result<(), PpuReferenceError> {
    let (status, reason) = match value {
        ReferenceField::Value { .. } => return Ok(()),
        ReferenceField::Undefined { reason } => ("undefined", reason),
        ReferenceField::Unsupported { reason } => ("unsupported", reason),
    };
    if reason.trim().is_empty() {
        return Err(PpuReferenceError::EmptyReason { field, status });
    }
    Ok(())
}

fn validate_provenance_text(field: &'static str, value: &str) -> Result<(), PpuReferenceError> {
    if value.trim().is_empty() {
        return Err(PpuReferenceError::EmptyProvenance { field });
    }
    Ok(())
}

fn compare_state(
    expected: &PpuReferenceState,
    observed: &PpuArchitecturalState,
    comparison: &mut PpuReferenceComparison,
) {
    compare_field(
        PpuReferenceComponent::StateGpr,
        &expected.gpr,
        &observed.gpr.to_vec(),
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateFpr,
        &expected.fpr,
        &observed.fpr.to_vec(),
        comparison,
    );
    let vr_hex = observed
        .vr
        .iter()
        .map(|value| format!("{value:032x}"))
        .collect::<Vec<_>>();
    compare_field(
        PpuReferenceComponent::StateVr,
        &expected.vr_hex,
        &vr_hex,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StatePc,
        &expected.pc,
        &observed.pc,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateCr,
        &expected.cr,
        &observed.cr,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateLr,
        &expected.lr,
        &observed.lr,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateCtr,
        &expected.ctr,
        &observed.ctr,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateXer,
        &expected.xer,
        &observed.xer,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateVrsave,
        &expected.vrsave,
        &observed.vrsave,
        comparison,
    );
    compare_field(
        PpuReferenceComponent::StateTb,
        &expected.tb,
        &observed.tb,
        comparison,
    );
    let reservation = observed.reservation.map(|line| line.addr());
    compare_field(
        PpuReferenceComponent::StateReservation,
        &expected.reservation,
        &reservation,
        comparison,
    );
}

fn compare_field<T: std::fmt::Debug + PartialEq>(
    field: PpuReferenceComponent,
    expected: &ReferenceField<T>,
    observed: &T,
    comparison: &mut PpuReferenceComparison,
) {
    match expected {
        ReferenceField::Value { value } => {
            comparison.compared.insert(field);
            if value != observed {
                comparison.differences.push(PpuReferenceDifference {
                    field,
                    expected: format!("{value:?}"),
                    observed: format!("{observed:?}"),
                });
            }
        }
        ReferenceField::Undefined { reason } => {
            comparison.unrepresented.push(PpuUnrepresentedField {
                field,
                status: PpuReferenceFieldStatus::Undefined,
                reason: reason.clone(),
            });
        }
        ReferenceField::Unsupported { reason } => {
            comparison.unrepresented.push(PpuUnrepresentedField {
                field,
                status: PpuReferenceFieldStatus::Unsupported,
                reason: reason.clone(),
            });
        }
    }
}

fn reference_yield(reason: YieldReason) -> PpuReferenceYieldReason {
    match reason {
        YieldReason::BudgetExhausted => PpuReferenceYieldReason::BudgetExhausted,
        YieldReason::MailboxAccess => PpuReferenceYieldReason::MailboxAccess,
        YieldReason::DmaSubmitted => PpuReferenceYieldReason::DmaSubmitted,
        YieldReason::DmaWait => PpuReferenceYieldReason::DmaWait,
        YieldReason::WaitingSync => PpuReferenceYieldReason::WaitingSync,
        YieldReason::Syscall => PpuReferenceYieldReason::Syscall,
        YieldReason::InterruptBoundary => PpuReferenceYieldReason::InterruptBoundary,
        YieldReason::Fault => PpuReferenceYieldReason::Fault,
        YieldReason::Finished => PpuReferenceYieldReason::Finished,
    }
}

fn render_effects(effects: &[Effect]) -> Vec<String> {
    effects.iter().map(|effect| format!("{effect:?}")).collect()
}
