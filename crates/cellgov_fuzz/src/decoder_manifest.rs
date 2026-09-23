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
pub const DECODER_MANIFEST_VERSION: u32 = 1;

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
        // [Chen2013 p:1 s:Abstract] Diverse failure-triggering cases are ranked ahead of repeated ones.
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
        let manifest = Self {
            schema_version: DECODER_MANIFEST_VERSION,
            raw_partitions,
            raw_totals,
            expected_kinds,
            witnesses,
            failures,
            expected_refusals,
            authoritative_references: Vec::new(),
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
        // [Jiang2022 p:1 s:Abstract] Device observations provide a comparison tier independent of emulator execution.
        let canonical = match decoder {
            RawDecoder::Ppu => serde_json::to_string(&parse_ppu_reference(json)?)?,
            RawDecoder::Spu => serde_json::to_string(&parse_spu_reference(json)?)?,
        };
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
        if !self.expected_kinds.windows(2).all(|pair| pair[0] < pair[1])
            || !self.witnesses.windows(2).all(|pair| {
                (
                    pair[0].kind.as_str(),
                    decoder_order(pair[0].replay.decoder),
                    pair[0].replay.raw,
                ) < (
                    pair[1].kind.as_str(),
                    decoder_order(pair[1].replay.decoder),
                    pair[1].replay.raw,
                )
            })
            || rank_failures(self.failures.clone()) != self.failures
            || self.witnesses.iter().any(|witness| {
                !self.expected_kinds.contains(&witness.kind)
                    || witness.classes.is_empty()
                    || !witness.classes.windows(2).all(|pair| pair[0] < pair[1])
            })
            || self.expected_kinds.iter().any(|kind| {
                !self.witnesses.iter().any(|witness| &witness.kind == kind)
                    && !self.failures.iter().any(|failure| {
                        failure.fingerprint.kind.as_ref() == Some(kind)
                            && matches!(
                                failure.fingerprint.class,
                                DecoderFailureClass::MissingKind
                                    | DecoderFailureClass::UnwitnessedKind
                            )
                    })
            })
            || self.failures.iter().any(|failure| {
                failure.minimized.is_some_and(|minimized| {
                    failure
                        .original
                        .is_none_or(|original| minimized.decoder != original.decoder)
                })
            })
            || self.failures.iter().any(|failure| {
                failure.original.is_none() == failure.descriptor_replay.is_none()
                    || failure.descriptor_replay.as_ref().is_some_and(|replay| {
                        failure.fingerprint.kind.as_ref() != Some(&replay.kind)
                    })
            })
            || !self.authoritative_references.windows(2).all(|pair| {
                (
                    decoder_order(pair[0].decoder),
                    pair[0].artifact_json.as_str(),
                ) < (
                    decoder_order(pair[1].decoder),
                    pair[1].artifact_json.as_str(),
                )
            })
        {
            return Err(DecoderManifestError::SemanticCoverage);
        }
        for reference in &self.authoritative_references {
            match reference.decoder {
                RawDecoder::Ppu => {
                    let artifact = parse_ppu_reference(&reference.artifact_json)?;
                    if serde_json::to_string(&artifact)? != reference.artifact_json {
                        return Err(DecoderManifestError::SemanticCoverage);
                    }
                }
                RawDecoder::Spu => {
                    let artifact = parse_spu_reference(&reference.artifact_json)?;
                    if serde_json::to_string(&artifact)? != reference.artifact_json {
                        return Err(DecoderManifestError::SemanticCoverage);
                    }
                }
            }
        }
        Ok(())
    }
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

fn merged_totals(
    artifacts: &[RawDecodeArtifact],
) -> Result<Vec<DecoderRawTotals>, DecoderManifestError> {
    let mut totals: BTreeMap<u8, DecoderRawTotals> = BTreeMap::new();
    let mut last_key = None;
    for artifact in artifacts {
        let key = (decoder_order(artifact.decoder), artifact.domain.first);
        // RawDecodeDomain::new requires a nonempty interval.
        if last_key.is_some_and(|last| key <= last)
            || artifact.domain.count == 0
            || artifact.schema_version != RAW_DECODE_SCHEMA_VERSION
            || artifact.status != RawDecodeStatus::Complete
            || artifact.processed != artifact.domain.count
            || artifact
                .accepted
                .checked_add(artifact.refused)
                .and_then(|count| count.checked_add(artifact.panics))
                != Some(artifact.domain.count)
            || artifact.panic_samples.len() as u64
                != artifact
                    .panics
                    .min(crate::raw_decode::MAX_RAW_DECODE_PANIC_SAMPLES as u64)
            || !artifact
                .panic_samples
                .windows(2)
                .all(|pair| pair[0].raw < pair[1].raw)
            || artifact.panic_samples.iter().any(|sample| {
                u64::from(sample.raw)
                    .checked_sub(u64::from(artifact.domain.first))
                    .is_none_or(|offset| offset >= artifact.domain.count)
            })
            || u64::from(artifact.domain.first)
                .checked_add(artifact.domain.count)
                .is_none_or(|end| end > u64::from(u32::MAX) + 1)
        {
            return Err(DecoderManifestError::RawPartitions);
        }
        let previous = totals.get_mut(&key.0);
        if let Some(total) = previous {
            let next = u64::from(total.first)
                .checked_add(total.words)
                .ok_or(DecoderManifestError::RawPartitions)?;
            if next != u64::from(artifact.domain.first) {
                return Err(DecoderManifestError::RawPartitions);
            }
            total.words = total
                .words
                .checked_add(artifact.domain.count)
                .ok_or(DecoderManifestError::RawPartitions)?;
            total.accepted = total
                .accepted
                .checked_add(artifact.accepted)
                .ok_or(DecoderManifestError::RawPartitions)?;
            total.refused = total
                .refused
                .checked_add(artifact.refused)
                .ok_or(DecoderManifestError::RawPartitions)?;
            total.panics = total
                .panics
                .checked_add(artifact.panics)
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
                },
            );
        }
        last_key = Some(key);
    }
    Ok(totals.into_values().collect())
}

fn semantic_failure(decoder: RawDecoder, finding: &SemanticSweepFinding) -> DecoderFailure {
    use SemanticSweepFinding as F;
    let stage = match finding {
        F::TargetPanic { stage, .. } => Some(*stage),
        _ => None,
    };
    let (class, kind, other_kind, raw, field, payload, case_class) = match finding {
        F::UnexpectedKind { kind } => (
            DecoderFailureClass::UnexpectedKind,
            Some(format!("{kind:?}")),
            None,
            None,
            None,
            None,
            None,
        ),
        F::DescriptorMismatch { kind, raw } => (
            DecoderFailureClass::DescriptorMismatch,
            Some(format!("{kind:?}")),
            None,
            Some(*raw),
            None,
            None,
            None,
        ),
        F::GenerationPanic { kind, payload } => (
            DecoderFailureClass::GenerationPanic,
            Some(format!("{kind:?}")),
            None,
            None,
            None,
            Some(payload.clone()),
            None,
        ),
        F::TargetPanic {
            kind, raw, payload, ..
        } => (
            DecoderFailureClass::TargetPanic,
            Some(format!("{kind:?}")),
            None,
            Some(*raw),
            None,
            Some(payload.clone()),
            None,
        ),
        F::MissingKind { kind } => (
            DecoderFailureClass::MissingKind,
            Some(format!("{kind:?}")),
            None,
            None,
            None,
            None,
            None,
        ),
        F::DuplicateKind { kind } => (
            DecoderFailureClass::DuplicateKind,
            Some(format!("{kind:?}")),
            None,
            None,
            None,
            None,
            None,
        ),
        F::UnwitnessedKind { kind } => (
            DecoderFailureClass::UnwitnessedKind,
            Some(format!("{kind:?}")),
            None,
            None,
            None,
            None,
            None,
        ),
        F::AmbiguousWord { raw, first, second } => (
            DecoderFailureClass::AmbiguousWord,
            Some(format!("{first:?}")),
            Some(format!("{second:?}")),
            Some(*raw),
            None,
            None,
            None,
        ),
        F::UnexpectedRejection { kind, raw, class } => (
            DecoderFailureClass::UnexpectedRejection,
            Some(format!("{kind:?}")),
            None,
            Some(*raw),
            None,
            None,
            Some(*class),
        ),
        F::UnexpectedAcceptance { kind, raw, field } => (
            DecoderFailureClass::UnexpectedAcceptance,
            Some(format!("{kind:?}")),
            None,
            Some(*raw),
            Some(*field),
            None,
            None,
        ),
        F::Misclassified {
            expected,
            actual,
            raw,
        } => (
            DecoderFailureClass::Misclassified,
            Some(format!("{expected:?}")),
            Some(format!("{actual:?}")),
            Some(*raw),
            None,
            None,
            None,
        ),
        F::RoundTripFailure { kind, raw, .. } => (
            DecoderFailureClass::RoundTripFailure,
            Some(format!("{kind:?}")),
            None,
            Some(*raw),
            None,
            None,
            None,
        ),
    };
    DecoderFailure {
        fingerprint: DecoderFailureFingerprint {
            class,
            kind: kind.clone(),
            other_kind,
            field,
            payload,
            stage,
            case_class,
        },
        original: raw.map(|raw| DecoderReplay { decoder, raw }),
        descriptor_replay: raw.is_none().then(|| DecoderDescriptorReplay {
            decoder,
            kind: kind.clone().unwrap_or_default(),
        }),
        minimized: None,
    }
}

/// Removes set bits while the caller confirms the exact same semantic fingerprint.
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
