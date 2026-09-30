//! The relation catalog: each kind declares exactly the relations the
//! catalog names for it, and each declared relation holds on the
//! interpreter over varied encodings and states.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use strum::VariantArray;

use super::metamorphic::relations_for_kind;
use super::registry::generation_descriptors;
use super::relations::{
    branch_complement, count_immediate_mask, count_register_bits, ignored_field_mask,
    immediate_register_pair, is_commutative, slot_permutation_reads_rt, CountBits, Splat,
};
use super::types::*;
use crate::instruction::{SpuInstruction, SpuInstructionKind};
use crate::observation::SpuObservation;
use crate::state::SpuState;

use SpuMetamorphicRelation as R;

/// A small deterministic generator for immediates and register values.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }
}

fn expected_relations(kind: SpuInstructionKind) -> Vec<SpuMetamorphicRelation> {
    let mut out = vec![R::Deterministic];
    if kind == SpuInstructionKind::Shufb {
        out.push(R::ShufbControlClass);
        return out;
    }
    let catalog = [
        (R::IgnoredField, ignored_field_mask(kind).is_some()),
        (
            R::CountMasking,
            count_immediate_mask(kind).is_some() || count_register_bits(kind).is_some(),
        ),
        (
            R::ImmediateRegister,
            immediate_register_pair(kind).is_some(),
        ),
        (R::Commutative, is_commutative(kind)),
        (
            R::SlotPermutation,
            slot_permutation_reads_rt(kind).is_some(),
        ),
        (R::CompareBranch, branch_complement(kind).is_some()),
    ];
    out.extend(
        catalog
            .into_iter()
            .filter(|(_, holds)| *holds)
            .map(|(r, _)| r),
    );
    out
}

#[test]
fn every_kind_declares_exactly_the_catalog_relations() {
    for &kind in SpuInstructionKind::VARIANTS {
        assert_eq!(
            relations_for_kind(kind),
            expected_relations(kind),
            "{kind:?}"
        );
    }
}

/// Words of `descriptor`'s kind with distinct registers, aliased registers,
/// and drawn immediates.
fn words(descriptor: &SpuGenerationDescriptor, rng: &mut Lcg) -> Vec<u32> {
    // Distinct registers, RT aliasing RA, RT aliasing RB, and RA in register 0,
    // the lowest register a varied input can take.
    let assignments: [[u32; 4]; 4] = [
        [10, 11, 12, 13],
        [11, 11, 12, 13],
        [12, 11, 12, 11],
        [1, 0, 2, 3],
    ];
    let mut out = Vec::new();
    for registers in assignments {
        for _ in 0..8 {
            let mut word = descriptor.canonical_word;
            let mut next_register = registers.iter();
            for field in &descriptor.operands {
                let shift = field.mask.trailing_zeros();
                let value = match field.class {
                    SpuOperandClass::Register => *next_register.next().unwrap_or(&14),
                    SpuOperandClass::Channel => {
                        descriptor.channel_values.first().copied().unwrap_or(0)
                    }
                    SpuOperandClass::Immediate => rng.next(),
                    SpuOperandClass::Flag => 0,
                };
                word = (word & !field.mask) | ((value << shift) & field.mask);
            }
            let decodes = crate::decode::decode(word)
                .is_ok_and(|instruction| SpuInstructionKind::from(instruction) == descriptor.kind);
            if decodes && !out.contains(&word) {
                out.push(word);
            }
        }
    }
    out
}

fn state(rng: &mut Lcg, zero: Option<u8>) -> SpuState {
    let mut state = SpuState::new();
    for register in 0..128 {
        state.set_reg(register, std::array::from_fn(|_| rng.next() as u8));
    }
    if let Some(register) = zero {
        state.set_reg(usize::from(register), [0; 16]);
    }
    state
}

fn observe(instruction: &SpuInstruction, initial: &SpuState) -> SpuObservation {
    let mut state = initial.clone();
    let outcome = crate::exec::execute(instruction, &mut state, UnitId::new(0));
    SpuObservation::capture(&state, &outcome)
}

/// The partner's observation, brought back to the frame of `original`.
fn observe_partner(
    case: &SpuMetamorphicCase,
    partner: &SpuInstruction,
    initial: &SpuState,
    original: &SpuObservation,
) -> SpuObservation {
    let mut state = case.partner_initial(initial);
    let outcome = crate::exec::execute(partner, &mut state, UnitId::new(0));
    let mut regs = *state.regs.as_array();
    case.settle_partner(&original.state.regs, &mut regs);
    state.set_reg_all(regs);
    SpuObservation::capture(&state, &outcome)
}

#[test]
fn every_declared_relation_holds_on_varied_words_and_states() {
    let mut rng = Lcg(0x1438);
    let mut witnessed: BTreeMap<(SpuInstructionKind, SpuMetamorphicRelation), u32> =
        BTreeMap::new();
    let mut failures = Vec::new();
    for descriptor in generation_descriptors() {
        let declared = relations_for_kind(descriptor.kind);
        for raw in words(&descriptor, &mut rng) {
            let instruction = crate::decode::decode(raw).expect("word decodes");
            for &relation in declared.iter().filter(|r| **r != R::Deterministic) {
                let Ok(case) = instruction.metamorphic_case(raw, relation) else {
                    continue;
                };
                let partner =
                    crate::decode::decode(case.partner_word).expect("an accepted partner decodes");
                for round in 0..6 {
                    // Every other state zeroes a register, so a branch on it is
                    // taken and a compare against it can hold.
                    let zero = (round % 2 == 1).then_some((raw & 0x7F) as u8);
                    let initial = state(&mut rng, zero);
                    let original = observe(&instruction, &initial);
                    let settled = observe_partner(&case, &partner, &initial, &original);
                    let differences = original.compare(&settled).differences;
                    if differences.is_empty() {
                        *witnessed.entry((descriptor.kind, relation)).or_default() += 1;
                    } else {
                        failures.push(format!(
                            "{:?} {relation:?} word {raw:#010x} partner {:#010x}: {differences:?}",
                            descriptor.kind, case.partner_word
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    for &kind in SpuInstructionKind::VARIANTS {
        for &relation in relations_for_kind(kind) {
            if relation != R::Deterministic {
                assert!(
                    witnessed.get(&(kind, relation)).copied().unwrap_or(0) > 0,
                    "{kind:?} declares {relation:?} with no witnessed partner"
                );
            }
        }
    }
}

#[test]
fn a_splat_extends_the_immediate_as_its_element_width_says() {
    // I10 = 0x3FF (-1) and I7 = 0x40 (-64), each in its own field.
    let i10 = 0x3FF << 14;
    let i7 = 0x40 << 14;
    assert_eq!(Splat::WordI10.value(i10), [0xFF; 16]);
    assert_eq!(Splat::HalfwordI10.value(i10), [0xFF; 16]);
    assert_eq!(Splat::ByteI10.value(0x2A5 << 14), [0xA5; 16]);
    assert_eq!(Splat::WordI7.value(i7)[0..4], (-64i32).to_be_bytes());
    assert_eq!(Splat::HalfwordI7.value(i7)[0..2], (-64i16).to_be_bytes());
    assert_eq!(
        Splat::WordI10.value(0x005 << 14)[12..16],
        5u32.to_be_bytes()
    );
}

#[test]
fn the_unread_count_bits_are_everything_but_the_count() {
    let word = CountBits::Word(0x3F).unread();
    assert_eq!(word[0..4], [0xFF, 0xFF, 0xFF, 0xC0]);
    assert_eq!(word[12..16], [0xFF, 0xFF, 0xFF, 0xC0]);
    assert_eq!(CountBits::Halfword(0x0F).unread()[0..2], [0xFF, 0xF0]);
    let preferred = CountBits::Preferred(0xF8).unread();
    assert_eq!(preferred[0..4], [0xFF, 0xFF, 0xFF, 0x07]);
    assert_eq!(preferred[4..16], [0xFF; 12]);
}
