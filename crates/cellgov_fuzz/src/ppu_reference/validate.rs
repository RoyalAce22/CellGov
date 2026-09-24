//! Parsing and validation of a PPU reference artifact, and its initial state.

use cellgov_ppu::state::PpuState;

use crate::reference::{is_lower_hex, ProvenanceFault, ReferenceField};

use super::types::{
    PpuReferenceArtifact, PpuReferenceError, PpuReferenceInputState, PpuReferenceObservation,
    PpuReferenceStateBase, PPU_REFERENCE_SCHEMA_VERSION,
};

const REFERENCE_DATA_BASE: u64 = 0x1000_0000;

/// Parses and validates a repository-data artifact.
// [Jiang2022 p:4 s:3] Cases come from the machine-readable specification, and a separate engine compares them on real devices and emulators.
pub fn parse_reference_json(json: &str) -> Result<PpuReferenceArtifact, PpuReferenceError> {
    let artifact: PpuReferenceArtifact = serde_json::from_str(json)?;
    artifact.validate()?;
    Ok(artifact)
}

impl PpuReferenceArtifact {
    pub(super) fn validate(&self) -> Result<(), PpuReferenceError> {
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
        self.provenance
            .check(|citation| {
                ["PPC-Book1", "PPC-Book2", "PPC-Book3", "AltiVec-PEM"]
                    .iter()
                    .any(|key| citation.starts_with(key))
                    && citation.contains(" p:")
                    && citation.contains(" s:")
            })
            .map_err(|fault| match fault {
                ProvenanceFault::Blank(field) => PpuReferenceError::EmptyProvenance { field },
                ProvenanceFault::Citation(citation) => PpuReferenceError::Citation {
                    citation: citation.to_owned(),
                },
                ProvenanceFault::Digest => PpuReferenceError::CaptureDigest,
            })
    }
}

impl PpuReferenceInputState {
    pub(super) fn to_state(&self) -> Result<PpuState, PpuReferenceError> {
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
    if !is_lower_hex(value, 32) {
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
    match value.blank_reason() {
        Some(omission) => Err(PpuReferenceError::EmptyReason {
            field,
            status: omission.as_str(),
        }),
        None => Ok(()),
    }
}
