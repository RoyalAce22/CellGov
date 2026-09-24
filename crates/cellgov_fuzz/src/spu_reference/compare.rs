//! Offline replay of an SPU reference and the comparison against the observed state.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::state::{SpuObservableSnapshot, SPU_REG_COUNT};

use crate::reference::{compare_field, ReferenceComparison, ReferenceField, ReferenceOmission};

use super::validate::{parse_index, parse_register};

use super::types::{
    SpuReferenceArtifact, SpuReferenceChannels, SpuReferenceComparison, SpuReferenceComponent,
    SpuReferenceError, SpuReferenceExpected, SpuReferenceOutcome, SpuReferenceReplay,
};

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
        &mut comparison,
        SpuReferenceComponent::Registers,
        &expected.regs_hex,
        |_| (registers != observed.regs).then_some(()),
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
        &mut comparison,
        SpuReferenceComponent::LocalStore,
        &expected.local_store,
        |_| (ls != observed.ls).then_some(()),
    );
    compare_field(
        &mut comparison,
        SpuReferenceComponent::ProgramCounter,
        &expected.pc,
        |value| (*value != observed.pc).then_some(()),
    );
    compare_field(
        &mut comparison,
        SpuReferenceComponent::Channels,
        &expected.channels,
        |value| (*value != SpuReferenceChannels::from(&observed.channels)).then_some(()),
    );
    compare_field(
        &mut comparison,
        SpuReferenceComponent::Reservation,
        &expected.reservation,
        |value| (*value != observed.reservation.map(|line| line.addr())).then_some(()),
    );
    compare_field(
        &mut comparison,
        SpuReferenceComponent::Outcome,
        &expected.outcome,
        |value| (*value != SpuReferenceOutcome::from(outcome)).then_some(()),
    );
    let effects_empty = match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects.is_empty(),
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_) => true,
    };
    compare_field(
        &mut comparison,
        SpuReferenceComponent::Effects,
        &expected.effects,
        |_| (!effects_empty).then_some(()),
    );
    compare_field(
        &mut comparison,
        SpuReferenceComponent::FaultDiscard,
        &expected.fault_discarded,
        |value| (*value != matches!(outcome, SpuStepOutcome::Fault(_))).then_some(()),
    );
    Ok(comparison)
}

impl ReferenceComparison for SpuReferenceComparison {
    type Component = SpuReferenceComponent;
    type Difference = ();

    fn compared(&mut self, component: SpuReferenceComponent) {
        self.compared.insert(component);
    }

    fn differs(&mut self, component: SpuReferenceComponent, _difference: ()) {
        self.differences.insert(component);
    }

    fn omitted(
        &mut self,
        component: SpuReferenceComponent,
        omission: ReferenceOmission,
        _reason: &str,
    ) {
        self.unrepresented.insert(component, omission);
    }
}
