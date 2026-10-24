//! Descriptor-owned instruction-kind and operand-class coverage.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_exec::operand::{OperandClass, OperandField};
use cellgov_ppu::instruction::fuzz::{
    expected_generation_kinds as expected_ppu_kinds, generation_descriptor as ppu_descriptor,
    generation_descriptors as ppu_descriptors, PpuGenerationDescriptor, PpuOperandClass,
};
use cellgov_spu::fuzz::{
    expected_generation_kinds as expected_spu_kinds, generation_descriptor as spu_descriptor,
    generation_descriptors as spu_descriptors, SpuGenerationDescriptor, SpuOperandClass,
};

use crate::boundary::call_target;
use crate::{InstructionIdentity, TargetPanicPayload};

/// Why the interpreter generated an instruction-word candidate.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCaseClass {
    /// Descriptor's known decodable word.
    Canonical,
    /// One operand at a typed boundary.
    OperandBoundary,
    /// Encoded register operands share one value.
    RegisterAlias,
    /// Immediate operand at a signed or unsigned boundary.
    ImmediateBoundary,
    /// Decoder-recognized non-operand bit variation.
    ReservedField,
}

/// One failure of descriptor coverage or decoder agreement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SemanticSweepFinding {
    /// A recipe names a kind outside the interpreter's decodable set.
    UnexpectedKind {
        /// Unrecognized declared kind.
        kind: InstructionIdentity,
    },
    /// A recipe's operand or encoding form differs from the decoder-derived recipe.
    DescriptorMismatch {
        /// Declared instruction identity.
        kind: InstructionIdentity,
        /// Word used to reconstruct the decoder-derived recipe.
        raw: u32,
    },
    /// Candidate generation panicked before it selected a word.
    GenerationPanic {
        /// Declared kind.
        kind: InstructionIdentity,
        /// Classified panic payload.
        payload: TargetPanicPayload,
    },
    /// The decoder or descriptor re-encoder panicked on a selected word.
    TargetPanic {
        /// Declared kind.
        kind: InstructionIdentity,
        /// Selected word.
        raw: u32,
        /// Comparison stage that panicked.
        stage: SemanticTargetStage,
        /// Classified panic payload.
        payload: TargetPanicPayload,
    },
    /// An expected kind has no descriptor.
    MissingKind {
        /// Interpreter-owned instruction identity.
        kind: InstructionIdentity,
    },
    /// A kind has more than one recipe.
    DuplicateKind {
        /// Duplicated identity.
        kind: InstructionIdentity,
    },
    /// A declared kind has no validated encoding.
    UnwitnessedKind {
        /// Declared identity.
        kind: InstructionIdentity,
    },
    /// Two distinct kinds claim the same raw word.
    AmbiguousWord {
        /// Two instruction kinds claim this word.
        raw: u32,
        /// First declared kind.
        first: InstructionIdentity,
        /// Second declared kind.
        second: InstructionIdentity,
    },
    /// Decoder refused a descriptor-accepted word.
    UnexpectedRejection {
        /// Declared identity.
        kind: InstructionIdentity,
        /// Rejected raw word.
        raw: u32,
        /// Generator class.
        class: SemanticCaseClass,
    },
    /// Structural encoder accepted an out-of-range operand.
    UnexpectedAcceptance {
        /// Declared identity.
        kind: InstructionIdentity,
        /// Out-of-range operand field index.
        field: usize,
        /// The structural encoder accepted this word.
        raw: u32,
    },
    /// Decoder named a different exact instruction kind.
    Misclassified {
        /// Declared identity.
        expected: InstructionIdentity,
        /// Decoded identity.
        actual: InstructionIdentity,
        /// The decoder assigns a different kind to this word.
        raw: u32,
    },
    /// A decodable instruction cannot re-encode its original word.
    RoundTripFailure {
        /// Decoded identity.
        kind: InstructionIdentity,
        /// Original word.
        raw: u32,
        /// Re-encoded result, if one was available.
        encoded: Option<u32>,
    },
}

/// Target boundary whose panic prevented a semantic comparison.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTargetStage {
    /// Decode the instruction word.
    Decoder,
    /// Re-encode the descriptor operands.
    DescriptorEncoder,
    /// Derive the operand contract from a decoded word.
    DescriptorMetadata,
}

/// One validated candidate and its independent generation classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticWitness {
    /// Exact decoded instruction identity.
    pub kind: InstructionIdentity,
    /// Word accepted by decoder and descriptor round-trip.
    pub raw: u32,
    /// Structural cases that produced this word.
    pub classes: BTreeSet<SemanticCaseClass>,
}

/// Authority of the encoder used for this sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticEncoderTier {
    /// Interpreter descriptors can re-encode their own represented operands.
    DescriptorRoundTrip,
}

/// Provenance tier of the comparison performed by semantic enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticObservationTier {
    /// Local decoder and descriptor agreement only; no external observation.
    DescriptorOnly,
}

/// Semantic enumeration result independent of raw 32-bit decoder coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSweepReport {
    /// Identifies which encoder contract was available for validation.
    pub encoder_tier: SemanticEncoderTier,
    /// Authoritative observations remain separate, versioned reference artifacts.
    pub observation_tier: SemanticObservationTier,
    /// Interpreter-owned kinds the registry must witness.
    pub expected_kinds: BTreeSet<InstructionIdentity>,
    /// Validated words in kind and word order.
    pub witnesses: Vec<SemanticWitness>,
    /// Distinct failure classes with replayable identities.
    pub findings: BTreeSet<SemanticSweepFinding>,
    /// Number of structurally invalid operands the encoder refused.
    pub expected_refusals: u64,
}

impl SemanticSweepReport {
    /// Reports a covered registry without losing any diagnostic class.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
            && self
                .expected_kinds
                .iter()
                .all(|kind| self.witnesses.iter().any(|witness| witness.kind == *kind))
    }
}

/// Compares PPU recipes with their decoder and interpreter-owned kind set.
pub fn sweep_ppu(descriptors: &[PpuGenerationDescriptor]) -> SemanticSweepReport {
    // [Jiang2022 p:5 s:3.1.1] Each encoding symbol gets a small mutation set: the boundary values of an immediate and a few fixed register indices.
    let expected = expected_ppu_kinds()
        .into_iter()
        .map(InstructionIdentity::Ppu)
        .collect();
    sweep_registry(expected, descriptors, |raw| {
        crate::seeded::ppu_decode(raw)
            .ok()
            .map(|instruction| InstructionIdentity::Ppu(instruction.fuzz_descriptor(raw).kind))
    })
}

/// Compares SPU recipes with their decoder and interpreter-owned kind set.
pub fn sweep_spu(descriptors: &[SpuGenerationDescriptor]) -> SemanticSweepReport {
    let expected = expected_spu_kinds()
        .into_iter()
        .map(InstructionIdentity::Spu)
        .collect();
    sweep_registry(expected, descriptors, |raw| {
        crate::seeded::spu_decode(raw)
            .ok()
            .map(|instruction| InstructionIdentity::Spu(instruction.into()))
    })
}

/// The sweep of one registry: every recipe's cases and out-of-range
/// probes against `decode` and the descriptor re-encoder, then each
/// recipe against the descriptor its canonical word derives.
fn sweep_registry<D: SweepDescriptor>(
    expected: BTreeSet<InstructionIdentity>,
    descriptors: &[D],
    decode: impl Fn(u32) -> Option<InstructionIdentity>,
) -> SemanticSweepReport {
    let mut report = sweep_descriptors(
        expected,
        descriptors.iter().map(|descriptor| {
            let kind = descriptor.identity();
            call_target(|| {
                (
                    classified_cases(descriptor),
                    out_of_range_probes(descriptor),
                )
            })
            .map(|(cases, rejected)| (kind, cases, rejected))
            .map_err(|payload| (kind, payload))
        }),
        decode,
        |raw| {
            D::derive(raw)
                .and_then(|descriptor| descriptor.encode(&descriptor.canonical_parameters()))
        },
    );
    check_descriptor_metadata(descriptors, &mut report);
    report
}

/// Record a finding for each recipe whose canonical word derives a
/// different recipe, or whose derivation panics.
fn check_descriptor_metadata<D: SweepDescriptor>(
    descriptors: &[D],
    report: &mut SemanticSweepReport,
) {
    for descriptor in descriptors {
        let kind = descriptor.identity();
        let raw = descriptor.canonical_word();
        match call_target(|| D::derive(raw)) {
            Ok(Some(actual)) if descriptor.same_recipe(&actual) => {}
            Ok(_) => {
                report
                    .findings
                    .insert(SemanticSweepFinding::DescriptorMismatch { kind, raw });
            }
            Err(payload) => {
                report.findings.insert(SemanticSweepFinding::TargetPanic {
                    kind,
                    raw,
                    stage: SemanticTargetStage::DescriptorMetadata,
                    payload,
                });
            }
        }
    }
}

/// The descriptor surface the sweep reads. The PPU and SPU generation
/// descriptors differ only in their kind and operand-class types.
trait SweepDescriptor: Sized {
    /// Operand class of this descriptor's fields.
    type Class: OperandClass;

    fn identity(&self) -> InstructionIdentity;
    fn canonical_word(&self) -> u32;
    fn operands(&self) -> &[OperandField<Self::Class>];
    fn canonical_parameters(&self) -> Vec<u32>;
    fn reserved_bit_words(&self) -> Vec<u32>;
    /// Pack `values` into the canonical word; `None` on a refusal.
    fn pack_operands(&self, values: &[u32]) -> Option<u32>;
    fn operands_are_defined(&self, word: u32) -> bool;
    /// Encode `values` as a valid word of this kind; `None` on a refusal.
    fn encode(&self, values: &[u32]) -> Option<u32>;
    /// The descriptor the decoder derives from `raw`.
    fn derive(raw: u32) -> Option<Self>;
    /// Whether `other` has this recipe's kind, form and operands.
    fn same_recipe(&self, other: &Self) -> bool;
}

impl SweepDescriptor for PpuGenerationDescriptor {
    type Class = PpuOperandClass;

    fn identity(&self) -> InstructionIdentity {
        InstructionIdentity::Ppu(self.kind)
    }
    fn canonical_word(&self) -> u32 {
        self.canonical_word
    }
    fn operands(&self) -> &[OperandField<PpuOperandClass>] {
        &self.operands
    }
    fn canonical_parameters(&self) -> Vec<u32> {
        PpuGenerationDescriptor::canonical_parameters(self)
    }
    fn reserved_bit_words(&self) -> Vec<u32> {
        PpuGenerationDescriptor::reserved_bit_words(self)
    }
    fn pack_operands(&self, values: &[u32]) -> Option<u32> {
        PpuGenerationDescriptor::pack_operands(self, values).ok()
    }
    fn operands_are_defined(&self, word: u32) -> bool {
        PpuGenerationDescriptor::operands_are_defined(self, word)
    }
    fn encode(&self, values: &[u32]) -> Option<u32> {
        PpuGenerationDescriptor::encode(self, values).ok()
    }
    fn derive(raw: u32) -> Option<Self> {
        ppu_descriptor(raw)
    }
    fn same_recipe(&self, other: &Self) -> bool {
        other.kind == self.kind && other.form == self.form && other.operands == self.operands
    }
}

impl SweepDescriptor for SpuGenerationDescriptor {
    type Class = SpuOperandClass;

    fn identity(&self) -> InstructionIdentity {
        InstructionIdentity::Spu(self.kind)
    }
    fn canonical_word(&self) -> u32 {
        self.canonical_word
    }
    fn operands(&self) -> &[OperandField<SpuOperandClass>] {
        &self.operands
    }
    fn canonical_parameters(&self) -> Vec<u32> {
        SpuGenerationDescriptor::canonical_parameters(self)
    }
    fn reserved_bit_words(&self) -> Vec<u32> {
        SpuGenerationDescriptor::reserved_bit_words(self)
    }
    fn pack_operands(&self, values: &[u32]) -> Option<u32> {
        SpuGenerationDescriptor::pack_operands(self, values).ok()
    }
    fn operands_are_defined(&self, word: u32) -> bool {
        SpuGenerationDescriptor::operands_are_defined(self, word)
    }
    fn encode(&self, values: &[u32]) -> Option<u32> {
        SpuGenerationDescriptor::encode(self, values).ok()
    }
    fn derive(raw: u32) -> Option<Self> {
        spu_descriptor(raw)
    }
    fn same_recipe(&self, other: &Self) -> bool {
        other.kind == self.kind && other.form == self.form && other.operands == self.operands
    }
}

/// Runs both descriptor registries without inventing opcode patterns.
pub fn sweep_both() -> (SemanticSweepReport, SemanticSweepReport) {
    (sweep_ppu(&ppu_descriptors()), sweep_spu(&spu_descriptors()))
}

type ClassifiedWords = BTreeMap<u32, BTreeSet<SemanticCaseClass>>;

fn insert(words: &mut ClassifiedWords, raw: u32, class: SemanticCaseClass) {
    words.entry(raw).or_default().insert(class);
}

fn classified_cases<D: SweepDescriptor>(descriptor: &D) -> ClassifiedWords {
    let mut words = ClassifiedWords::new();
    insert(
        &mut words,
        descriptor.canonical_word(),
        SemanticCaseClass::Canonical,
    );
    for word in descriptor.reserved_bit_words() {
        insert(&mut words, word, SemanticCaseClass::ReservedField);
    }
    insert_register_aliases(descriptor, &mut words);
    let canonical = descriptor.canonical_parameters();
    for (index, field) in descriptor.operands().iter().enumerate() {
        for value in field.boundary_values() {
            let mut params = canonical.clone();
            params[index] = value;
            let Some(word) = descriptor.pack_operands(&params) else {
                continue;
            };
            if !descriptor.operands_are_defined(word) {
                continue;
            }
            insert(&mut words, word, SemanticCaseClass::OperandBoundary);
            if field.class == D::Class::IMMEDIATE {
                insert(&mut words, word, SemanticCaseClass::ImmediateBoundary);
            }
        }
    }
    words
}

/// Every register field set to one shared value, for each of a few
/// values. A recipe with no register field has no alias case.
fn insert_register_aliases<D: SweepDescriptor>(descriptor: &D, words: &mut ClassifiedWords) {
    let has_register = descriptor
        .operands()
        .iter()
        .any(|field| field.class == D::Class::REGISTER);
    if !has_register {
        return;
    }
    for value in [0, 7, 127] {
        let mut params = descriptor.canonical_parameters();
        for (param, field) in params.iter_mut().zip(descriptor.operands()) {
            if field.class == D::Class::REGISTER {
                *param = value & field.maximum();
            }
        }
        let Some(word) = descriptor.pack_operands(&params) else {
            continue;
        };
        if descriptor.operands_are_defined(word) {
            insert(words, word, SemanticCaseClass::RegisterAlias);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvalidProbe {
    Refused,
    Accepted { field: usize, raw: u32 },
}

fn out_of_range_probes<D: SweepDescriptor>(descriptor: &D) -> Vec<InvalidProbe> {
    descriptor
        .operands()
        .iter()
        .enumerate()
        .filter(|(_, field)| field.maximum() < u32::MAX)
        .map(|(index, field)| {
            let mut params = descriptor.canonical_parameters();
            params[index] = field.maximum() + 1;
            match descriptor.encode(&params) {
                Some(raw) => InvalidProbe::Accepted { field: index, raw },
                None => InvalidProbe::Refused,
            }
        })
        .collect()
}

fn sweep_descriptors(
    expected_kinds: BTreeSet<InstructionIdentity>,
    descriptors: impl Iterator<
        Item = Result<
            (InstructionIdentity, ClassifiedWords, Vec<InvalidProbe>),
            (InstructionIdentity, TargetPanicPayload),
        >,
    >,
    decode: impl Fn(u32) -> Option<InstructionIdentity>,
    encode: impl Fn(u32) -> Option<u32>,
) -> SemanticSweepReport {
    let mut report = SemanticSweepReport {
        encoder_tier: SemanticEncoderTier::DescriptorRoundTrip,
        // [Jiang2022 p:6 s:3.2.2] An inconsistent stream is one whose final state on the emulator differs
        // from the real device's. Local decoder and descriptor agreement never compares against a device,
        // so it cannot claim that tier.
        observation_tier: SemanticObservationTier::DescriptorOnly,
        expected_kinds,
        witnesses: Vec::new(),
        findings: BTreeSet::new(),
        expected_refusals: 0,
    };
    let mut kinds = BTreeSet::new();
    let mut claimed_words = BTreeMap::new();
    for generated in descriptors {
        let (kind, words, probe) = match generated {
            Ok(candidate) => candidate,
            Err((kind, payload)) => {
                kinds.insert(kind);
                report
                    .findings
                    .insert(SemanticSweepFinding::GenerationPanic { kind, payload });
                continue;
            }
        };
        if !report.expected_kinds.contains(&kind) {
            report
                .findings
                .insert(SemanticSweepFinding::UnexpectedKind { kind });
        }
        if !kinds.insert(kind) {
            report
                .findings
                .insert(SemanticSweepFinding::DuplicateKind { kind });
        }
        for probe in probe {
            match probe {
                InvalidProbe::Accepted { field, raw } => {
                    report
                        .findings
                        .insert(SemanticSweepFinding::UnexpectedAcceptance { kind, field, raw });
                }
                InvalidProbe::Refused => report.expected_refusals += 1,
            }
        }
        for (raw, classes) in words {
            if let Some(&first) = claimed_words.get(&raw) {
                if first != kind {
                    report.findings.insert(SemanticSweepFinding::AmbiguousWord {
                        raw,
                        first,
                        second: kind,
                    });
                }
            } else {
                claimed_words.insert(raw, kind);
            }
            let decoded = match call_target(|| decode(raw)) {
                Ok(decoded) => decoded,
                Err(payload) => {
                    report.findings.insert(SemanticSweepFinding::TargetPanic {
                        kind,
                        raw,
                        stage: SemanticTargetStage::Decoder,
                        payload,
                    });
                    continue;
                }
            };
            let encoded = match call_target(|| encode(raw)) {
                Ok(encoded) => encoded,
                Err(payload) => {
                    report.findings.insert(SemanticSweepFinding::TargetPanic {
                        kind,
                        raw,
                        stage: SemanticTargetStage::DescriptorEncoder,
                        payload,
                    });
                    continue;
                }
            };
            for &class in &classes {
                classify_candidate(kind, raw, class, decoded, encoded, &mut report);
            }
            if decoded == Some(kind) && encoded == Some(raw) {
                report
                    .witnesses
                    .push(SemanticWitness { kind, raw, classes });
            }
        }
    }
    for &kind in &report.expected_kinds {
        if !kinds.contains(&kind) {
            report
                .findings
                .insert(SemanticSweepFinding::MissingKind { kind });
        } else if !report.witnesses.iter().any(|witness| witness.kind == kind) {
            report
                .findings
                .insert(SemanticSweepFinding::UnwitnessedKind { kind });
        }
    }
    report
        .witnesses
        .sort_by_key(|witness| (witness.kind, witness.raw));
    report
}

fn classify_candidate(
    kind: InstructionIdentity,
    raw: u32,
    class: SemanticCaseClass,
    decoded: Option<InstructionIdentity>,
    encoded: Option<u32>,
    report: &mut SemanticSweepReport,
) {
    match decoded {
        None => {
            report
                .findings
                .insert(SemanticSweepFinding::UnexpectedRejection { kind, raw, class });
        }
        Some(actual) if actual != kind => {
            report.findings.insert(SemanticSweepFinding::Misclassified {
                expected: kind,
                actual,
                raw,
            });
        }
        Some(_) => {}
    }
    if decoded.is_some() && encoded != Some(raw) {
        report
            .findings
            .insert(SemanticSweepFinding::RoundTripFailure { kind, raw, encoded });
    }
}

#[cfg(test)]
#[path = "tests/semantic_sweep_tests.rs"]
mod tests;
