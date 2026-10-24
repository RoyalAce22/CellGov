//! Versioned coverage and replay records for raw and semantic decoder campaigns.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ppu_reference::{parse_reference_json as parse_ppu_reference, PpuReferenceError};
use crate::raw_decode::{
    RawDecodeArtifact, RawDecodeStatus, RawDecoder, RAW_DECODE_SCHEMA_VERSION,
};
use crate::semantic_sweep::{
    SemanticCaseClass, SemanticSweepFinding, SemanticSweepReport, SemanticTargetStage,
};
use crate::spu_reference::{parse_reference_json as parse_spu_reference, SpuReferenceError};
use crate::{InstructionIdentity, TargetPanicPayload};

/// Version of the decoder campaign manifest.
pub const DECODER_MANIFEST_VERSION: u32 = 2;

/// Stable replay coordinate for one decoder case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderReplay {
    /// Interpreter decoder to invoke.
    pub decoder: RawDecoder,
    /// Original 32-bit instruction word.
    pub raw: u32,
}

/// Descriptor lookup coordinate for a finding without an instruction word.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderDescriptorReplay {
    /// Interpreter whose descriptor registry to inspect.
    pub decoder: RawDecoder,
    /// Exact interpreter-owned instruction identity.
    pub kind: String,
}

/// Decoder result without dependence on formatted diagnostic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoderFailureClass {
    /// Interpreter kind absent from the generator.
    MissingKind,
    /// Generator declares one kind more than once.
    DuplicateKind,
    /// Generator declares a kind outside the interpreter set.
    UnexpectedKind,
    /// Declared kind has no validated word.
    UnwitnessedKind,
    /// Descriptor differs from the decoder-derived contract.
    DescriptorMismatch,
    /// Two kinds claim one word.
    AmbiguousWord,
    /// Decoder rejects a generated word.
    UnexpectedRejection,
    /// Encoder accepts an out-of-range operand.
    UnexpectedAcceptance,
    /// Decoder assigns another kind.
    Misclassified,
    /// Decoder and descriptor encoder disagree.
    RoundTripFailure,
    /// Candidate generation panics.
    GenerationPanic,
    /// Interpreter or descriptor metadata panics.
    TargetPanic,
    /// Raw decoder panics.
    RawPanic,
    /// The disassembly of a decoded word names another mnemonic.
    DisassemblyDisagreement,
}

/// Machine-readable semantic identity for failure grouping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderFailureFingerprint {
    /// Typed failure category.
    pub class: DecoderFailureClass,
    /// First declared interpreter kind, if any.
    pub kind: Option<String>,
    /// Conflicting interpreter kind, if any.
    pub other_kind: Option<String>,
    /// Operand field index, if any.
    pub field: Option<usize>,
    /// Panic payload classification, if any.
    pub payload: Option<TargetPanicPayload>,
    /// Failing stage when the descriptor or decoder panics.
    pub stage: Option<SemanticTargetStage>,
    /// Structural class whose classification must stay unchanged.
    pub case_class: Option<SemanticCaseClass>,
}

/// Original failing input and a smaller same-class replay if available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderFailure {
    /// Stable classification, independent of rendered diagnostics.
    pub fingerprint: DecoderFailureFingerprint,
    /// Exact input that first exposed the failure.
    pub original: Option<DecoderReplay>,
    /// Descriptor identity to replay when no raw word was available.
    pub descriptor_replay: Option<DecoderDescriptorReplay>,
    /// Same-class replay obtained by deterministic bit deletion.
    pub minimized: Option<DecoderReplay>,
}

impl DecoderFailure {
    /// Retains the original word while reducing a same-class replay.
    pub fn localize(
        &mut self,
        classify: impl Fn(DecoderReplay) -> Option<DecoderFailureFingerprint>,
    ) -> Option<DecoderReplay> {
        self.minimized = None;
        let original = self.original?;
        let reduced = minimize_decoder_failure(original, &self.fingerprint, classify)?;
        self.minimized = Some(reduced);
        Some(reduced)
    }
}

/// One interpreter-owned instruction witness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderWitness {
    /// Exact instruction-kind identity.
    pub kind: String,
    /// Replayable instruction word.
    pub replay: DecoderReplay,
    /// Structural classes represented by the witness.
    pub classes: Vec<SemanticCaseClass>,
}

/// Aggregate counts from a contiguous sequence of raw partitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderRawTotals {
    /// Interpreter decoder scanned by the partitions.
    pub decoder: RawDecoder,
    /// First raw word in the merged interval.
    pub first: u32,
    /// Number of words in the interval.
    pub words: u64,
    /// Decoder acceptances.
    pub accepted: u64,
    /// Decoder refusals.
    pub refused: u64,
    /// Decoder panics, including unretained samples.
    pub panics: u64,
    /// Decoded words whose disassembly disagrees with the decoder.
    pub disagreements: u64,
}

/// What one SPU instruction word does when the SPU meets it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuRowOutcome {
    /// The decoder builds this instruction kind.
    Decoded {
        /// Exact instruction-kind identity, as the witnesses name it.
        kind: String,
    },
    /// The SPU stops with the invalid-instruction status bit set.
    InvalidInstructionStop,
}

/// One SPU instruction word and what it does: a witness for one row of
/// the SPU opcode map, or for the words no row selects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuRowWitness {
    /// The row's mnemonic; `None` for a word no row selects.
    pub mnemonic: Option<String>,
    /// The word: the row's opcode with every other field zero.
    pub raw: u32,
    /// What the word does.
    pub outcome: SpuRowOutcome,
}

/// Validated independent vector or capture, separate from local decoder checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderReferenceEvidence {
    /// Interpreter whose vector or capture supplies the observation.
    pub decoder: RawDecoder,
    /// Canonical versioned source artifact with citation or capture provenance.
    pub artifact_json: String,
}

/// Versioned snapshot of decoder classification and structural coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderCampaignManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Raw partitions in decoder and word order.
    pub raw_partitions: Vec<RawDecodeArtifact>,
    /// Exact merged totals for each decoder.
    pub raw_totals: Vec<DecoderRawTotals>,
    /// Interpreter-owned instruction identities.
    pub expected_kinds: Vec<String>,
    /// Semantic witnesses in kind and word order.
    pub witnesses: Vec<DecoderWitness>,
    /// Findings grouped by semantic fingerprint with original replay preserved.
    pub failures: Vec<DecoderFailure>,
    /// Number of out-of-range operand probes refused.
    pub expected_refusals: u64,
    /// Independent reference artifacts, never inferred from descriptor agreement.
    pub authoritative_references: Vec<DecoderReferenceEvidence>,
    /// One witness per row of the SPU opcode map, in map order.
    pub spu_rows: Vec<SpuRowWitness>,
    /// A witness for the SPU words no row selects.
    pub spu_unassigned: SpuRowWitness,
}

/// Refusal to construct or validate a decoder campaign manifest.
#[derive(Debug, thiserror::Error)]
pub enum DecoderManifestError {
    /// Reports manifest JSON syntax errors.
    #[error("decoder manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Unsupported manifest version.
    #[error("decoder manifest version {found} is unsupported")]
    Version {
        /// Artifact version.
        found: u32,
    },
    /// Raw partitions overlap, have a gap, or carry invalid totals.
    #[error("decoder manifest raw partitions are incomplete or inconsistent")]
    RawPartitions,
    /// Semantic identities, witnesses, or findings contradict each other.
    #[error("decoder manifest semantic coverage is inconsistent")]
    SemanticCoverage,
    /// Current coverage differs from its committed baseline.
    #[error("decoder manifest coverage differs from the expected snapshot")]
    CoverageDrift,
    /// PPU vector or capture has invalid content or provenance.
    #[error("decoder manifest PPU reference is invalid: {0}")]
    PpuReference(#[from] PpuReferenceError),
    /// SPU vector or capture has invalid content or provenance.
    #[error("decoder manifest SPU reference is invalid: {0}")]
    SpuReference(#[from] SpuReferenceError),
}

impl DecoderCampaignManifest {
    /// Builds a deterministic manifest from complete raw partitions and semantic reports.
    ///
    /// # Errors
    ///
    /// Refuses invalid, cancelled, overlapping, or noncontiguous raw partitions.
    pub fn build(
        raw_partitions: &[RawDecodeArtifact],
        ppu: &SemanticSweepReport,
        spu: &SemanticSweepReport,
    ) -> Result<Self, DecoderManifestError> {
        // [Chen2013 p:2 s:2.1 Definitions] The manifest ranks failures so that any
        // prefix of the list covers as many distinct defects as possible. Repeats
        // of one fingerprint fall behind the first case of every other.
        let mut raw_partitions = raw_partitions.to_vec();
        raw_partitions
            .sort_by_key(|artifact| (decoder_order(artifact.decoder), artifact.domain.first));
        let raw_totals = merged_totals(&raw_partitions)?;
        let mut expected_kinds = Vec::new();
        let mut witnesses = Vec::new();
        let mut failures = Vec::new();
        let mut expected_refusals = 0u64;
        for (decoder, report) in [(RawDecoder::Ppu, ppu), (RawDecoder::Spu, spu)] {
            // Replay coordinates must name the interpreter that supplied the kind.
            if report
                .expected_kinds
                .iter()
                .any(|&kind| !identity_matches_decoder(kind, decoder))
                || report
                    .witnesses
                    .iter()
                    .any(|witness| !identity_matches_decoder(witness.kind, decoder))
                || report
                    .findings
                    .iter()
                    .any(|finding| !finding_matches_decoder(finding, decoder))
            {
                return Err(DecoderManifestError::SemanticCoverage);
            }
            expected_kinds.extend(report.expected_kinds.iter().map(|kind| format!("{kind:?}")));
            witnesses.extend(report.witnesses.iter().map(|witness| DecoderWitness {
                kind: format!("{:?}", witness.kind),
                replay: DecoderReplay {
                    decoder,
                    raw: witness.raw,
                },
                classes: witness.classes.iter().copied().collect(),
            }));
            failures.extend(
                report
                    .findings
                    .iter()
                    .map(|finding| semantic_failure(decoder, finding)),
            );
            expected_refusals = expected_refusals
                .checked_add(report.expected_refusals)
                .ok_or(DecoderManifestError::SemanticCoverage)?;
        }
        for artifact in &raw_partitions {
            failures.extend(
                artifact
                    .disagreement_samples
                    .iter()
                    .map(|&raw| DecoderFailure {
                        fingerprint: DecoderFailureFingerprint {
                            class: DecoderFailureClass::DisassemblyDisagreement,
                            kind: None,
                            other_kind: None,
                            field: None,
                            payload: None,
                            stage: None,
                            case_class: None,
                        },
                        original: Some(DecoderReplay {
                            decoder: artifact.decoder,
                            raw,
                        }),
                        descriptor_replay: None,
                        minimized: None,
                    }),
            );
            failures.extend(artifact.panic_samples.iter().map(|sample| DecoderFailure {
                fingerprint: DecoderFailureFingerprint {
                    class: DecoderFailureClass::RawPanic,
                    kind: None,
                    other_kind: None,
                    field: None,
                    payload: Some(sample.payload.clone()),
                    stage: None,
                    case_class: None,
                },
                original: Some(DecoderReplay {
                    decoder: artifact.decoder,
                    raw: sample.raw,
                }),
                descriptor_replay: None,
                minimized: None,
            }));
        }
        expected_kinds.sort();
        witnesses.sort_by_key(|witness| {
            (
                witness.kind.clone(),
                decoder_order(witness.replay.decoder),
                witness.replay.raw,
            )
        });
        failures = rank_failures(failures);
        let (spu_rows, spu_unassigned) =
            spu_row_witnesses().ok_or(DecoderManifestError::SemanticCoverage)?;
        let manifest = Self {
            schema_version: DECODER_MANIFEST_VERSION,
            raw_partitions,
            raw_totals,
            expected_kinds,
            witnesses,
            failures,
            expected_refusals,
            authoritative_references: Vec::new(),
            spu_rows,
            spu_unassigned,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Parses and validates a versioned manifest without trusting its stored totals.
    ///
    /// # Errors
    ///
    /// Refuses unsupported versions and contradictory or reordered content.
    pub fn parse_json(json: &str) -> Result<Self, DecoderManifestError> {
        let value: Self = serde_json::from_str(json)?;
        value.validate()?;
        Ok(value)
    }

    /// Rejects a lost witness, changed classification, or changed raw total.
    ///
    /// # Errors
    ///
    /// Refuses any difference from the committed baseline.
    pub fn check_coverage(&self, baseline: &Self) -> Result<(), DecoderManifestError> {
        self.validate()?;
        baseline.validate()?;
        (self == baseline)
            .then_some(())
            .ok_or(DecoderManifestError::CoverageDrift)
    }

    /// Adds a validated versioned source without treating local agreement as external evidence.
    ///
    /// # Errors
    ///
    /// Refuses invalid artifacts or duplicate source identities.
    pub fn attach_reference(
        &mut self,
        decoder: RawDecoder,
        json: &str,
    ) -> Result<(), DecoderManifestError> {
        // [Jiang2022 p:4 s:2.2.3 Differential Testing] A word executed on a real
        // device is a comparison tier independent of the emulator. Only that
        // kind of record counts as external evidence here.
        let canonical = canonical_reference(decoder, json)?;
        if self
            .authoritative_references
            .iter()
            .any(|reference| reference.decoder == decoder && reference.artifact_json == canonical)
        {
            return Err(DecoderManifestError::SemanticCoverage);
        }
        self.authoritative_references
            .push(DecoderReferenceEvidence {
                decoder,
                artifact_json: canonical,
            });
        self.authoritative_references.sort_by_key(|reference| {
            (
                decoder_order(reference.decoder),
                reference.artifact_json.clone(),
            )
        });
        Ok(())
    }

    fn validate(&self) -> Result<(), DecoderManifestError> {
        if self.schema_version != DECODER_MANIFEST_VERSION {
            return Err(DecoderManifestError::Version {
                found: self.schema_version,
            });
        }
        if merged_totals(&self.raw_partitions)? != self.raw_totals {
            return Err(DecoderManifestError::RawPartitions);
        }
        if !SEMANTIC_CHECKS.iter().all(|check| check(self)) {
            return Err(DecoderManifestError::SemanticCoverage);
        }
        for reference in &self.authoritative_references {
            if canonical_reference(reference.decoder, &reference.artifact_json)?
                != reference.artifact_json
            {
                return Err(DecoderManifestError::SemanticCoverage);
            }
        }
        Ok(())
    }
}

/// The semantic half of [`DecoderCampaignManifest::validate`]: each
/// check names one property a well-formed manifest holds.
const SEMANTIC_CHECKS: [fn(&DecoderCampaignManifest) -> bool; 9] = [
    DecoderCampaignManifest::expected_kinds_sorted,
    DecoderCampaignManifest::witnesses_sorted,
    DecoderCampaignManifest::failures_ranked,
    DecoderCampaignManifest::witness_classes_valid,
    DecoderCampaignManifest::every_kind_witnessed_or_failed,
    DecoderCampaignManifest::minimized_on_original_decoder,
    DecoderCampaignManifest::replay_shape_consistent,
    DecoderCampaignManifest::references_sorted,
    DecoderCampaignManifest::spu_rows_are_complete,
];

impl DecoderCampaignManifest {
    fn expected_kinds_sorted(&self) -> bool {
        self.expected_kinds.windows(2).all(|pair| pair[0] < pair[1])
    }

    /// Witnesses in kind, decoder, word order, with no repeat.
    fn witnesses_sorted(&self) -> bool {
        fn key(witness: &DecoderWitness) -> (&str, u8, u32) {
            (
                witness.kind.as_str(),
                decoder_order(witness.replay.decoder),
                witness.replay.raw,
            )
        }
        self.witnesses
            .windows(2)
            .all(|pair| key(&pair[0]) < key(&pair[1]))
    }

    fn failures_ranked(&self) -> bool {
        rank_failures(self.failures.clone()) == self.failures
    }

    /// Each witness names an expected kind and carries a nonempty,
    /// strictly sorted class list.
    fn witness_classes_valid(&self) -> bool {
        self.witnesses.iter().all(|witness| {
            self.expected_kinds.contains(&witness.kind)
                && !witness.classes.is_empty()
                && witness.classes.windows(2).all(|pair| pair[0] < pair[1])
        })
    }

    /// Each expected kind has a witness or a missing-kind or
    /// unwitnessed-kind failure.
    fn every_kind_witnessed_or_failed(&self) -> bool {
        self.expected_kinds.iter().all(|kind| {
            self.witnesses.iter().any(|witness| &witness.kind == kind)
                || self.failures.iter().any(|failure| {
                    failure.fingerprint.kind.as_ref() == Some(kind)
                        && matches!(
                            failure.fingerprint.class,
                            DecoderFailureClass::MissingKind | DecoderFailureClass::UnwitnessedKind
                        )
                })
        })
    }

    /// A minimized replay runs on the decoder of its original word.
    fn minimized_on_original_decoder(&self) -> bool {
        self.failures.iter().all(|failure| {
            failure.minimized.is_none_or(|minimized| {
                failure
                    .original
                    .is_some_and(|original| minimized.decoder == original.decoder)
            })
        })
    }

    /// Each failure has exactly one of an original word and a descriptor
    /// replay, and a descriptor replay names the fingerprint's kind.
    fn replay_shape_consistent(&self) -> bool {
        self.failures.iter().all(|failure| {
            failure.original.is_none() != failure.descriptor_replay.is_none()
                && failure
                    .descriptor_replay
                    .as_ref()
                    .is_none_or(|replay| failure.fingerprint.kind.as_ref() == Some(&replay.kind))
        })
    }

    fn references_sorted(&self) -> bool {
        self.authoritative_references.windows(2).all(|pair| {
            (
                decoder_order(pair[0].decoder),
                pair[0].artifact_json.as_str(),
            ) < (
                decoder_order(pair[1].decoder),
                pair[1].artifact_json.as_str(),
            )
        })
    }
}

/// `json` parsed as `decoder`'s reference artifact and serialized again.
fn canonical_reference(decoder: RawDecoder, json: &str) -> Result<String, DecoderManifestError> {
    Ok(match decoder {
        RawDecoder::Ppu => serde_json::to_string(&parse_ppu_reference(json)?)?,
        RawDecoder::Spu => serde_json::to_string(&parse_spu_reference(json)?)?,
    })
}

impl DecoderCampaignManifest {
    /// Whether every row of the SPU opcode map has its witness, in map
    /// order:
    ///
    /// - a row the CBE provides is a decoded kind the manifest witnesses;
    /// - a row the CBE lacks is an invalid-instruction stop;
    /// - the unassigned witness selects no row and is an
    ///   invalid-instruction stop.
    fn spu_rows_are_complete(&self) -> bool {
        use cellgov_ps3_abi::hw::spu_isa::{row_for, SPU_OPCODE_MAP};
        self.spu_rows.len() == SPU_OPCODE_MAP.len()
            && self
                .spu_rows
                .iter()
                .zip(SPU_OPCODE_MAP)
                .all(|(witness, row)| {
                    witness.mnemonic.as_deref() == Some(row.mnemonic)
                        && witness.raw == row.canonical_word()
                        && match (&witness.outcome, row.on_cbe) {
                            (SpuRowOutcome::Decoded { kind }, true) => self
                                .witnesses
                                .iter()
                                .any(|w| &w.kind == kind && w.replay.decoder == RawDecoder::Spu),
                            (SpuRowOutcome::InvalidInstructionStop, false) => true,
                            _ => false,
                        }
                })
            && self.spu_unassigned.mnemonic.is_none()
            && row_for(self.spu_unassigned.raw).is_none()
            && self.spu_unassigned.outcome == SpuRowOutcome::InvalidInstructionStop
    }
}

/// A witness for every row of the SPU opcode map, and one for the first
/// word no row selects, or `None` when a word does neither of the two
/// things the map allows: decode, or stop the SPU as an invalid
/// instruction.
fn spu_row_witnesses() -> Option<(Vec<SpuRowWitness>, SpuRowWitness)> {
    use cellgov_ps3_abi::hw::spu_isa::{row_for, SPU_OPCODE_MAP};
    let rows = SPU_OPCODE_MAP
        .iter()
        .map(|row| {
            let raw = row.canonical_word();
            Some(SpuRowWitness {
                mnemonic: Some(row.mnemonic.to_string()),
                raw,
                outcome: spu_word_outcome(raw)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let unassigned = (0..1u32 << 11)
        .map(|opcode| opcode << 21)
        .find(|&raw| row_for(raw).is_none())?;
    let unassigned = SpuRowWitness {
        mnemonic: None,
        raw: unassigned,
        outcome: spu_word_outcome(unassigned)?,
    };
    Some((rows, unassigned))
}

/// What `raw` does: the kind it decodes to, or, run as the first word of
/// a fresh SPU, an invalid-instruction stop. `None` for anything else.
fn spu_word_outcome(raw: u32) -> Option<SpuRowOutcome> {
    use cellgov_exec::ExecutionUnit;
    if let Ok(instruction) = cellgov_spu::decode::decode(raw) {
        let kind = cellgov_spu::instruction::SpuInstructionKind::from(instruction);
        return Some(SpuRowOutcome::Decoded {
            kind: format!("{:?}", InstructionIdentity::Spu(kind)),
        });
    }
    // [CBEA p:93 s:8.5.2] I: invalid instruction detected, SPU stopped.
    let mut unit = cellgov_spu::SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    unit.state_mut().ls[..4].copy_from_slice(&raw.to_be_bytes());
    let memory = cellgov_mem::GuestMemory::new(16);
    let context = cellgov_exec::ExecutionContext::new(&memory);
    let _ = unit.run_until_yield(cellgov_time::Budget::new(1), &context, &mut Vec::new());
    unit.stop_registers()
        .is_some_and(|stop| stop.status & cellgov_ps3_abi::hw::spu::SPU_STATUS_I != 0)
        .then_some(SpuRowOutcome::InvalidInstructionStop)
}

fn decoder_order(decoder: RawDecoder) -> u8 {
    match decoder {
        RawDecoder::Ppu => 0,
        RawDecoder::Spu => 1,
    }
}

fn identity_matches_decoder(kind: InstructionIdentity, decoder: RawDecoder) -> bool {
    matches!(
        (kind, decoder),
        (InstructionIdentity::Ppu(_), RawDecoder::Ppu)
            | (InstructionIdentity::Spu(_), RawDecoder::Spu)
    )
}

fn finding_matches_decoder(finding: &SemanticSweepFinding, decoder: RawDecoder) -> bool {
    use SemanticSweepFinding as F;
    let kind_matches = |kind| identity_matches_decoder(kind, decoder);
    match finding {
        F::UnexpectedKind { kind }
        | F::DescriptorMismatch { kind, .. }
        | F::GenerationPanic { kind, .. }
        | F::TargetPanic { kind, .. }
        | F::MissingKind { kind }
        | F::DuplicateKind { kind }
        | F::UnwitnessedKind { kind }
        | F::UnexpectedRejection { kind, .. }
        | F::UnexpectedAcceptance { kind, .. }
        | F::RoundTripFailure { kind, .. } => kind_matches(*kind),
        F::AmbiguousWord { first, second, .. } => kind_matches(*first) && kind_matches(*second),
        F::Misclassified { expected, .. } => kind_matches(*expected),
    }
}

fn rank_failures(failures: Vec<DecoderFailure>) -> Vec<DecoderFailure> {
    let mut buckets: BTreeMap<DecoderFailureFingerprint, Vec<DecoderFailure>> = BTreeMap::new();
    for failure in failures {
        buckets
            .entry(failure.fingerprint.clone())
            .or_default()
            .push(failure);
    }
    for bucket in buckets.values_mut() {
        bucket.sort_by_key(|failure| {
            (
                failure.original,
                failure.descriptor_replay.clone(),
                failure.minimized,
            )
        });
    }
    let mut ranked = Vec::new();
    let longest = buckets.values().map(Vec::len).max().unwrap_or(0);
    for position in 0..longest {
        for bucket in buckets.values() {
            if let Some(failure) = bucket.get(position) {
                ranked.push(failure.clone());
            }
        }
    }
    ranked
}

/// Whether one complete raw partition is internally consistent: its
/// counts add up, its domain fits the 32-bit word space, and its panic
/// and disagreement samples are the right number, strictly sorted and
/// inside the domain.
fn raw_artifact_is_well_formed(artifact: &RawDecodeArtifact) -> bool {
    use crate::raw_decode::{MAX_RAW_DECODE_DISAGREEMENT_SAMPLES, MAX_RAW_DECODE_PANIC_SAMPLES};
    let domain = &artifact.domain;
    let in_domain = |raw: u32| {
        u64::from(raw)
            .checked_sub(u64::from(domain.first))
            .is_some_and(|offset| offset < domain.count)
    };
    let counts_add_up = artifact.schema_version == RAW_DECODE_SCHEMA_VERSION
        && artifact.status == RawDecodeStatus::Complete
        && artifact.processed == domain.count
        && artifact
            .accepted
            .checked_add(artifact.refused)
            .and_then(|count| count.checked_add(artifact.panics))
            == Some(domain.count)
        && artifact.disagreements <= artifact.accepted;
    // RawDecodeDomain::new requires a nonempty interval.
    let domain_fits = domain.count != 0
        && u64::from(domain.first)
            .checked_add(domain.count)
            .is_some_and(|end| end <= u64::from(u32::MAX) + 1);
    let panics_sampled = artifact.panic_samples.len() as u64
        == artifact.panics.min(MAX_RAW_DECODE_PANIC_SAMPLES as u64)
        && artifact
            .panic_samples
            .windows(2)
            .all(|pair| pair[0].raw < pair[1].raw)
        && artifact
            .panic_samples
            .iter()
            .all(|sample| in_domain(sample.raw));
    let disagreements_sampled = artifact.disagreement_samples.len() as u64
        == artifact
            .disagreements
            .min(MAX_RAW_DECODE_DISAGREEMENT_SAMPLES as u64)
        && artifact
            .disagreement_samples
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        && artifact
            .disagreement_samples
            .iter()
            .all(|&raw| in_domain(raw));
    counts_add_up && domain_fits && panics_sampled && disagreements_sampled
}

impl DecoderRawTotals {
    /// Add the next partition of the same decoder, which starts where
    /// these totals end. `None` on a gap, an overlap or an overflow.
    fn absorb(&mut self, artifact: &RawDecodeArtifact) -> Option<()> {
        let next = u64::from(self.first).checked_add(self.words)?;
        if next != u64::from(artifact.domain.first) {
            return None;
        }
        self.words = self.words.checked_add(artifact.domain.count)?;
        self.accepted = self.accepted.checked_add(artifact.accepted)?;
        self.refused = self.refused.checked_add(artifact.refused)?;
        self.panics = self.panics.checked_add(artifact.panics)?;
        self.disagreements = self.disagreements.checked_add(artifact.disagreements)?;
        Some(())
    }
}

fn merged_totals(
    artifacts: &[RawDecodeArtifact],
) -> Result<Vec<DecoderRawTotals>, DecoderManifestError> {
    let mut totals: BTreeMap<u8, DecoderRawTotals> = BTreeMap::new();
    let mut last_key = None;
    for artifact in artifacts {
        let key = (decoder_order(artifact.decoder), artifact.domain.first);
        if last_key.is_some_and(|last| key <= last) || !raw_artifact_is_well_formed(artifact) {
            return Err(DecoderManifestError::RawPartitions);
        }
        if let Some(total) = totals.get_mut(&key.0) {
            total
                .absorb(artifact)
                .ok_or(DecoderManifestError::RawPartitions)?;
        } else {
            totals.insert(
                key.0,
                DecoderRawTotals {
                    decoder: artifact.decoder,
                    first: artifact.domain.first,
                    words: artifact.domain.count,
                    accepted: artifact.accepted,
                    refused: artifact.refused,
                    panics: artifact.panics,
                    disagreements: artifact.disagreements,
                },
            );
        }
        last_key = Some(key);
    }
    Ok(totals.into_values().collect())
}

fn semantic_failure(decoder: RawDecoder, finding: &SemanticSweepFinding) -> DecoderFailure {
    use DecoderFailureClass as C;
    use SemanticSweepFinding as F;
    let fingerprint = match finding {
        F::UnexpectedKind { kind } => base_fingerprint(C::UnexpectedKind, kind),
        F::DescriptorMismatch { kind, .. } => base_fingerprint(C::DescriptorMismatch, kind),
        F::GenerationPanic { kind, payload } => DecoderFailureFingerprint {
            payload: Some(payload.clone()),
            ..base_fingerprint(C::GenerationPanic, kind)
        },
        F::TargetPanic {
            kind,
            payload,
            stage,
            ..
        } => DecoderFailureFingerprint {
            payload: Some(payload.clone()),
            stage: Some(*stage),
            ..base_fingerprint(C::TargetPanic, kind)
        },
        F::MissingKind { kind } => base_fingerprint(C::MissingKind, kind),
        F::DuplicateKind { kind } => base_fingerprint(C::DuplicateKind, kind),
        F::UnwitnessedKind { kind } => base_fingerprint(C::UnwitnessedKind, kind),
        F::AmbiguousWord { first, second, .. } => DecoderFailureFingerprint {
            other_kind: Some(format!("{second:?}")),
            ..base_fingerprint(C::AmbiguousWord, first)
        },
        F::UnexpectedRejection { kind, class, .. } => DecoderFailureFingerprint {
            case_class: Some(*class),
            ..base_fingerprint(C::UnexpectedRejection, kind)
        },
        F::UnexpectedAcceptance { kind, field, .. } => DecoderFailureFingerprint {
            field: Some(*field),
            ..base_fingerprint(C::UnexpectedAcceptance, kind)
        },
        F::Misclassified {
            expected, actual, ..
        } => DecoderFailureFingerprint {
            other_kind: Some(format!("{actual:?}")),
            ..base_fingerprint(C::Misclassified, expected)
        },
        F::RoundTripFailure { kind, .. } => base_fingerprint(C::RoundTripFailure, kind),
    };
    let raw = finding_word(finding);
    DecoderFailure {
        original: raw.map(|raw| DecoderReplay { decoder, raw }),
        descriptor_replay: raw.is_none().then(|| DecoderDescriptorReplay {
            decoder,
            kind: fingerprint.kind.clone().unwrap_or_default(),
        }),
        fingerprint,
        minimized: None,
    }
}

/// A fingerprint of `class` for `kind` with every optional field empty.
fn base_fingerprint(
    class: DecoderFailureClass,
    kind: &InstructionIdentity,
) -> DecoderFailureFingerprint {
    DecoderFailureFingerprint {
        class,
        kind: Some(format!("{kind:?}")),
        other_kind: None,
        field: None,
        payload: None,
        stage: None,
        case_class: None,
    }
}

/// The word a finding names, if it names one.
fn finding_word(finding: &SemanticSweepFinding) -> Option<u32> {
    use SemanticSweepFinding as F;
    match finding {
        F::DescriptorMismatch { raw, .. }
        | F::TargetPanic { raw, .. }
        | F::AmbiguousWord { raw, .. }
        | F::UnexpectedRejection { raw, .. }
        | F::UnexpectedAcceptance { raw, .. }
        | F::Misclassified { raw, .. }
        | F::RoundTripFailure { raw, .. } => Some(*raw),
        F::UnexpectedKind { .. }
        | F::GenerationPanic { .. }
        | F::MissingKind { .. }
        | F::DuplicateKind { .. }
        | F::UnwitnessedKind { .. } => None,
    }
}

/// Removes set bits while the caller confirms the exact same semantic fingerprint.
///
/// Each cleared bit is one shortening step; the reducer keeps it when the
/// fingerprint holds and restores it when the fingerprint changes.
/// [McKeeman1998 p:105 s:Test Reduction]
pub fn minimize_decoder_failure(
    original: DecoderReplay,
    fingerprint: &DecoderFailureFingerprint,
    classify: impl Fn(DecoderReplay) -> Option<DecoderFailureFingerprint>,
) -> Option<DecoderReplay> {
    if classify(original).as_ref() != Some(fingerprint) {
        return None;
    }
    let mut reduced = original;
    for bit in (0..u32::BITS).rev() {
        let candidate = DecoderReplay {
            raw: reduced.raw & !(1u32 << bit),
            ..reduced
        };
        if candidate.raw != reduced.raw && classify(candidate).as_ref() == Some(fingerprint) {
            reduced = candidate;
        }
    }
    Some(reduced)
}

#[cfg(test)]
#[path = "tests/decoder_manifest_tests.rs"]
mod tests;
