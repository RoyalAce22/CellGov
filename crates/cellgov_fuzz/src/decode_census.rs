//! Exhaustive classification of PPU instruction words against the decoder,
//! the encoder and the gap tables, with a per-opcode coverage histogram.

use std::collections::BTreeMap;

use cellgov_ppu::instruction::encode::{alias, reserved_bits, Alias};
use cellgov_ppu::instruction::known_encodings::{opcode_gap, spr_gap, SprDirection};
use cellgov_ppu::instruction::{Locator, PpuDecodeError, PpuInstruction};
use serde::{Deserialize, Serialize};

use crate::boundary::call_target;
use crate::raw_decode::{RawDecodeDomain, RawDecodeError};

/// Version of the census artifact.
pub const DECODE_CENSUS_SCHEMA_VERSION: u32 = 1;
/// Maximum finding samples one artifact retains.
pub const MAX_DECODE_CENSUS_SAMPLES: usize = 128;

/// How the decoder, the encoder and the gap tables classify one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordClass {
    /// Decoded, and the encoder returns the same word.
    Canonical,
    /// Decoded, and the word differs from its canonical form only in bits the decoder does not read.
    ReservedBits,
    /// Decoded through a second spelling the encoder names.
    Alias,
    /// Decoded, but the encoder refuses it, its word decodes to another instruction, or the difference from the canonical word has no reserved-bit or alias explanation.
    RoundTripFailure,
    /// Rejected as a documented encoding with no decoder arm, and the gap tables name the same mnemonic.
    ArmUnimplemented,
    /// Rejected as a documented encoding whose mnemonic the gap tables do not name for its locator.
    ArmUnlisted,
    /// Rejected as no documented encoding.
    NotRecognized,
    /// The decoder or the encoder panicked.
    Panic,
}

/// A word that breaks one of the census properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CensusFinding {
    /// The word decodes but does not round-trip through the encoder.
    RoundTripFailure,
    /// The rejection names a mnemonic the gap tables do not carry.
    ArmUnlisted,
    /// The decoder or the encoder panicked.
    Panic,
    /// A word under primary opcode 0 decoded.
    PrimaryZeroDecoded,
}

/// One retained finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CensusSample {
    /// Instruction word.
    pub raw: u32,
    /// Property the word breaks.
    pub finding: CensusFinding,
}

/// Word counts by class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassCounts {
    /// Words the encoder reproduces exactly.
    pub canonical: u64,
    /// Words that differ from canonical only in unread bits.
    pub reserved_bits: u64,
    /// Words decoded through a second spelling.
    pub alias: u64,
    /// Decoded words the encoder cannot reproduce.
    pub round_trip_failures: u64,
    /// Documented encodings without a decoder arm.
    pub arm_unimplemented: u64,
    /// Rejections whose mnemonic the gap tables do not name.
    pub arm_unlisted: u64,
    /// Words that match no documented encoding.
    pub not_recognized: u64,
    /// Words that made the decoder or encoder panic.
    pub panics: u64,
}

impl ClassCounts {
    fn total(&self) -> Option<u64> {
        [
            self.canonical,
            self.reserved_bits,
            self.alias,
            self.round_trip_failures,
            self.arm_unimplemented,
            self.arm_unlisted,
            self.not_recognized,
            self.panics,
        ]
        .iter()
        .try_fold(0u64, |sum, count| sum.checked_add(*count))
    }

    fn add(&mut self, other: &Self) -> Option<()> {
        self.canonical = self.canonical.checked_add(other.canonical)?;
        self.reserved_bits = self.reserved_bits.checked_add(other.reserved_bits)?;
        self.alias = self.alias.checked_add(other.alias)?;
        self.round_trip_failures = self
            .round_trip_failures
            .checked_add(other.round_trip_failures)?;
        self.arm_unimplemented = self
            .arm_unimplemented
            .checked_add(other.arm_unimplemented)?;
        self.arm_unlisted = self.arm_unlisted.checked_add(other.arm_unlisted)?;
        self.not_recognized = self.not_recognized.checked_add(other.not_recognized)?;
        self.panics = self.panics.checked_add(other.panics)?;
        Some(())
    }

    fn count(&mut self, class: WordClass) {
        let slot = match class {
            WordClass::Canonical => &mut self.canonical,
            WordClass::ReservedBits => &mut self.reserved_bits,
            WordClass::Alias => &mut self.alias,
            WordClass::RoundTripFailure => &mut self.round_trip_failures,
            WordClass::ArmUnimplemented => &mut self.arm_unimplemented,
            WordClass::ArmUnlisted => &mut self.arm_unlisted,
            WordClass::NotRecognized => &mut self.not_recognized,
            WordClass::Panic => &mut self.panics,
        };
        *slot += 1;
    }
}

/// Outcome counts for one opcode bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketCounts {
    /// Words that decoded.
    pub decoded: u64,
    /// Rejections that name a documented encoding.
    pub arm_unimplemented: u64,
    /// Rejections that name no documented encoding.
    pub not_recognized: u64,
    /// Panics.
    pub panics: u64,
}

impl BucketCounts {
    fn count(&mut self, class: WordClass) {
        let slot = match class {
            WordClass::Canonical
            | WordClass::ReservedBits
            | WordClass::Alias
            | WordClass::RoundTripFailure => &mut self.decoded,
            WordClass::ArmUnimplemented | WordClass::ArmUnlisted => &mut self.arm_unimplemented,
            WordClass::NotRecognized => &mut self.not_recognized,
            WordClass::Panic => &mut self.panics,
        };
        *slot += 1;
    }

    fn add(&mut self, other: &Self) -> Option<()> {
        self.decoded = self.decoded.checked_add(other.decoded)?;
        self.arm_unimplemented = self
            .arm_unimplemented
            .checked_add(other.arm_unimplemented)?;
        self.not_recognized = self.not_recognized.checked_add(other.not_recognized)?;
        self.panics = self.panics.checked_add(other.panics)?;
        Some(())
    }

    fn total(&self) -> Option<u64> {
        self.decoded
            .checked_add(self.arm_unimplemented)?
            .checked_add(self.not_recognized)?
            .checked_add(self.panics)
    }
}

/// Outcome counts for one primary opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimaryCensus {
    /// Primary opcode, 0 to 63.
    pub primary: u8,
    /// Counts over every word under the primary.
    pub counts: BucketCounts,
}

/// Outcome counts for one extended opcode under a primary that has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtendedCensus {
    /// Primary opcode.
    pub primary: u8,
    /// Extended opcode at the primary's widest width: eleven bits under 4,
    /// ten under 19, 31, 59 and 63, four under 30. A narrower form spreads
    /// across the buckets its operand bits select.
    pub xo: u16,
    /// Counts over every word with this primary and extended opcode.
    pub counts: BucketCounts,
}

/// Versioned result of a census over one word interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeCensusArtifact {
    /// Artifact schema version.
    pub schema_version: u32,
    /// Word interval the census covers.
    pub domain: RawDecodeDomain,
    /// Counts by class over the interval.
    pub classes: ClassCounts,
    /// Counts by primary opcode, ascending, for the primaries the interval touches.
    pub primaries: Vec<PrimaryCensus>,
    /// Counts by extended opcode under primaries 4, 19, 30, 31, 59 and 63, ascending.
    pub extended: Vec<ExtendedCensus>,
    /// First finding words, ascending, at most [`MAX_DECODE_CENSUS_SAMPLES`].
    pub samples: Vec<CensusSample>,
}

/// Typed refusal to build, merge or parse a census.
#[derive(Debug, thiserror::Error)]
pub enum DecodeCensusError {
    /// Artifact JSON could not be decoded.
    #[error("decode census artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Artifact uses an unsupported schema version.
    #[error("decode census schema version {found} is unsupported; expected {supported}")]
    Version {
        /// Artifact version.
        found: u32,
        /// Supported version.
        supported: u32,
    },
    /// Empty or overflowing domain.
    #[error("decode census domain is invalid: {0}")]
    Domain(#[source] RawDecodeError),
    /// Counts, histograms or samples contradict the domain.
    #[error("decode census artifact has inconsistent counts, histograms or samples")]
    InvalidArtifact,
    /// A merge received no parts.
    #[error("decode census merge received no parts")]
    NoParts,
    /// Merge parts do not tile one contiguous interval.
    #[error("decode census parts are not contiguous: expected a part starting at 0x{expected:08x}, found 0x{found:08x}")]
    Discontiguous {
        /// Word the next part had to start at.
        expected: u64,
        /// Word the next part started at.
        found: u32,
    },
    /// A counter cannot represent the merged total.
    #[error("decode census counters overflowed")]
    CounterOverflow,
}

/// Classifies one word.
pub fn classify(raw: u32) -> WordClass {
    match call_target(|| crate::seeded::ppu_decode(raw)) {
        Err(_) => WordClass::Panic,
        Ok(Err(PpuDecodeError::DecoderArmUnimplemented {
            locator, mnemonic, ..
        })) => {
            if gap_mnemonic(locator) == Some(mnemonic) {
                WordClass::ArmUnimplemented
            } else {
                WordClass::ArmUnlisted
            }
        }
        Ok(Err(PpuDecodeError::EncodingNotRecognized { .. })) => WordClass::NotRecognized,
        Ok(Ok(instruction)) => {
            call_target(|| round_trip(raw, instruction)).unwrap_or(WordClass::Panic)
        }
    }
}

fn gap_mnemonic(locator: Locator) -> Option<&'static str> {
    match locator {
        Locator::Opcode { primary, xo } => opcode_gap(primary, xo).map(|gap| gap.mnemonic),
        Locator::Spr { op_mnemonic, spr } => {
            [SprDirection::MfSpr, SprDirection::MfTb, SprDirection::MtSpr]
                .into_iter()
                .find(|direction| direction.op_mnemonic() == op_mnemonic)
                .and_then(|direction| spr_gap(direction, spr))
                .map(|gap| gap.mnemonic)
        }
    }
}

fn round_trip(raw: u32, instruction: PpuInstruction) -> WordClass {
    let Ok(word) = crate::seeded::ppu_encode(&instruction) else {
        return WordClass::RoundTripFailure;
    };
    if crate::seeded::ppu_decode(word) != Ok(instruction) {
        return WordClass::RoundTripFailure;
    }
    if word == raw {
        return WordClass::Canonical;
    }
    let explained = match alias(raw) {
        None => return classify_reserved(raw, word, &instruction),
        Some(Alias::NopHint { .. }) => {
            instruction
                == PpuInstruction::Ori {
                    ra: 0,
                    rs: 0,
                    imm: 0,
                }
        }
        Some(Alias::TimeBaseThroughMfspr { tbr }) => matches!(
            (tbr, instruction),
            (268, PpuInstruction::Mftb { .. }) | (269, PpuInstruction::Mftbu { .. })
        ),
    };
    if explained {
        WordClass::Alias
    } else {
        WordClass::RoundTripFailure
    }
}

fn classify_reserved(raw: u32, canonical: u32, instruction: &PpuInstruction) -> WordClass {
    if raw & !reserved_bits(instruction) == canonical {
        WordClass::ReservedBits
    } else {
        WordClass::RoundTripFailure
    }
}

/// Mask of the widest extended-opcode field a keyed primary uses.
fn extended_xo_limit(primary: u8) -> Option<u16> {
    // [PPC-Book1 p:7 s:1.7 Instruction formats] XO width and position are form-dependent.
    match primary {
        // [AltiVec-PEM p:A-21 s:A.5] VX-form XO is the low eleven bits; VA-form uses the low six.
        4 => Some(0x7FF),
        // [PPC-Book1 p:9 s:1.7.6 X-Form] XO(21:30) under primaries 19, 31, 59 and 63.
        // [PPC-Book1 p:10 s:1.7.12 A-Form] XO(26:30) under 59 and 63, with FRC in the bits above it.
        19 | 31 | 59 | 63 => Some(0x3FF),
        // [PPC-Book1 p:10 s:1.7.14 MD-Form] XO(27:29) with bit 30 for the MDS split.
        30 => Some(0xF),
        _ => None,
    }
}

/// Extended-opcode key of a word, for the primaries that carry one.
pub fn extended_key(raw: u32) -> Option<(u8, u16)> {
    let primary = (raw >> 26) as u8;
    let limit = extended_xo_limit(primary)?;
    // Primary 4 keys on the low bits; every other keyed primary skips Rc or LK at bit 31.
    let field = if primary == 4 { raw } else { raw >> 1 };
    Some((primary, (field & u32::from(limit)) as u16))
}

/// Runs the census over one interval, word by word in ascending order.
///
/// # Errors
///
/// Refuses an empty or overflowing interval.
pub fn census(domain: RawDecodeDomain) -> Result<DecodeCensusArtifact, DecodeCensusError> {
    RawDecodeDomain::new(domain.first, domain.count).map_err(DecodeCensusError::Domain)?;
    let mut classes = ClassCounts::default();
    let mut primaries: BTreeMap<u8, BucketCounts> = BTreeMap::new();
    let mut extended: BTreeMap<(u8, u16), BucketCounts> = BTreeMap::new();
    let mut samples = Vec::new();
    for offset in 0..domain.count {
        let raw = (u64::from(domain.first) + offset) as u32;
        let class = classify(raw);
        classes.count(class);
        let primary = (raw >> 26) as u8;
        primaries.entry(primary).or_default().count(class);
        if let Some(key) = extended_key(raw) {
            extended.entry(key).or_default().count(class);
        }
        let finding = match class {
            WordClass::RoundTripFailure => Some(CensusFinding::RoundTripFailure),
            WordClass::ArmUnlisted => Some(CensusFinding::ArmUnlisted),
            WordClass::Panic => Some(CensusFinding::Panic),
            WordClass::Canonical | WordClass::ReservedBits | WordClass::Alias if primary == 0 => {
                Some(CensusFinding::PrimaryZeroDecoded)
            }
            _ => None,
        };
        if let Some(finding) = finding {
            if samples.len() < MAX_DECODE_CENSUS_SAMPLES {
                samples.push(CensusSample { raw, finding });
            }
        }
    }
    let artifact = DecodeCensusArtifact {
        schema_version: DECODE_CENSUS_SCHEMA_VERSION,
        domain,
        classes,
        primaries: primaries
            .into_iter()
            .map(|(primary, counts)| PrimaryCensus { primary, counts })
            .collect(),
        extended: extended
            .into_iter()
            .map(|((primary, xo), counts)| ExtendedCensus {
                primary,
                xo,
                counts,
            })
            .collect(),
        samples,
    };
    artifact.validate()?;
    Ok(artifact)
}

/// Merges parts that tile one contiguous interval, in any order.
///
/// # Errors
///
/// Refuses no parts, a gap or overlap between parts, an inconsistent
/// part, or a counter overflow.
pub fn merge(parts: &[DecodeCensusArtifact]) -> Result<DecodeCensusArtifact, DecodeCensusError> {
    let mut ordered: Vec<&DecodeCensusArtifact> = parts.iter().collect();
    ordered.sort_by_key(|part| part.domain.first);
    let first = ordered.first().ok_or(DecodeCensusError::NoParts)?;
    let mut classes = ClassCounts::default();
    let mut primaries: BTreeMap<u8, BucketCounts> = BTreeMap::new();
    let mut extended: BTreeMap<(u8, u16), BucketCounts> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut next = u64::from(first.domain.first);
    let mut count = 0u64;
    for part in &ordered {
        part.validate()?;
        if u64::from(part.domain.first) != next {
            return Err(DecodeCensusError::Discontiguous {
                expected: next,
                found: part.domain.first,
            });
        }
        next = next
            .checked_add(part.domain.count)
            .ok_or(DecodeCensusError::CounterOverflow)?;
        count = count
            .checked_add(part.domain.count)
            .ok_or(DecodeCensusError::CounterOverflow)?;
        classes
            .add(&part.classes)
            .ok_or(DecodeCensusError::CounterOverflow)?;
        for row in &part.primaries {
            primaries
                .entry(row.primary)
                .or_default()
                .add(&row.counts)
                .ok_or(DecodeCensusError::CounterOverflow)?;
        }
        for row in &part.extended {
            extended
                .entry((row.primary, row.xo))
                .or_default()
                .add(&row.counts)
                .ok_or(DecodeCensusError::CounterOverflow)?;
        }
        samples.extend(part.samples.iter().copied());
    }
    samples.truncate(MAX_DECODE_CENSUS_SAMPLES);
    let domain =
        RawDecodeDomain::new(first.domain.first, count).map_err(DecodeCensusError::Domain)?;
    let artifact = DecodeCensusArtifact {
        schema_version: DECODE_CENSUS_SCHEMA_VERSION,
        domain,
        classes,
        primaries: primaries
            .into_iter()
            .map(|(primary, counts)| PrimaryCensus { primary, counts })
            .collect(),
        extended: extended
            .into_iter()
            .map(|((primary, xo), counts)| ExtendedCensus {
                primary,
                xo,
                counts,
            })
            .collect(),
        samples,
    };
    artifact.validate()?;
    Ok(artifact)
}

impl DecodeCensusArtifact {
    /// Parses a versioned census and validates its counts.
    ///
    /// # Errors
    ///
    /// Refuses malformed JSON, an unsupported version, or inconsistent counts.
    pub fn parse_json(json: &str) -> Result<Self, DecodeCensusError> {
        let artifact: Self = serde_json::from_str(json)?;
        if artifact.schema_version != DECODE_CENSUS_SCHEMA_VERSION {
            return Err(DecodeCensusError::Version {
                found: artifact.schema_version,
                supported: DECODE_CENSUS_SCHEMA_VERSION,
            });
        }
        artifact.validate()?;
        Ok(artifact)
    }

    /// Words that decoded under primary opcode 0.
    pub fn primary_zero_decoded(&self) -> u64 {
        self.primaries
            .iter()
            .find(|row| row.primary == 0)
            .map_or(0, |row| row.counts.decoded)
    }

    /// Words that break a census property.
    pub fn findings(&self) -> u64 {
        self.classes.round_trip_failures
            + self.classes.arm_unlisted
            + self.classes.panics
            + self.primary_zero_decoded()
    }

    /// Reports an interval with every property held.
    pub fn is_clean(&self) -> bool {
        self.findings() == 0
    }

    fn validate(&self) -> Result<(), DecodeCensusError> {
        if self.schema_version != DECODE_CENSUS_SCHEMA_VERSION {
            return Err(DecodeCensusError::Version {
                found: self.schema_version,
                supported: DECODE_CENSUS_SCHEMA_VERSION,
            });
        }
        RawDecodeDomain::new(self.domain.first, self.domain.count)
            .map_err(DecodeCensusError::Domain)?;
        // Checked sums come first: `findings` adds unchecked, so it runs
        // only once every count is known to fit the domain.
        let mut by_primary = BucketCounts::default();
        for row in &self.primaries {
            by_primary
                .add(&row.counts)
                .ok_or(DecodeCensusError::InvalidArtifact)?;
        }
        let mut by_extended: BTreeMap<u8, BucketCounts> = BTreeMap::new();
        for row in &self.extended {
            by_extended
                .entry(row.primary)
                .or_default()
                .add(&row.counts)
                .ok_or(DecodeCensusError::InvalidArtifact)?;
        }
        let decoded = self
            .classes
            .canonical
            .checked_add(self.classes.reserved_bits)
            .and_then(|sum| sum.checked_add(self.classes.alias))
            .and_then(|sum| sum.checked_add(self.classes.round_trip_failures));
        let rejected = self
            .classes
            .arm_unimplemented
            .checked_add(self.classes.arm_unlisted);
        let classes_match_primaries = decoded == Some(by_primary.decoded)
            && rejected == Some(by_primary.arm_unimplemented)
            && self.classes.not_recognized == by_primary.not_recognized
            && self.classes.panics == by_primary.panics;
        if self.classes.total() != Some(self.domain.count)
            || by_primary.total() != Some(self.domain.count)
            || !classes_match_primaries
        {
            return Err(DecodeCensusError::InvalidArtifact);
        }
        let primaries_sorted = self
            .primaries
            .windows(2)
            .all(|pair| pair[0].primary < pair[1].primary);
        let extended_sorted = self
            .extended
            .windows(2)
            .all(|pair| (pair[0].primary, pair[0].xo) < (pair[1].primary, pair[1].xo));
        let extended_under_their_primaries = self.extended.iter().all(|row| {
            extended_xo_limit(row.primary).is_some_and(|limit| row.xo <= limit)
                && self
                    .primaries
                    .iter()
                    .any(|primary| primary.primary == row.primary)
        });
        let extended_tile_their_primaries = self.primaries.iter().all(|row| {
            extended_xo_limit(row.primary).is_none()
                || by_extended.get(&row.primary).copied().unwrap_or_default() == row.counts
        });
        let samples_in_domain = self.samples.iter().all(|sample| {
            u64::from(sample.raw)
                .checked_sub(u64::from(self.domain.first))
                .is_some_and(|offset| offset < self.domain.count)
        });
        let samples_sorted = self
            .samples
            .windows(2)
            .all(|pair| pair[0].raw < pair[1].raw);
        let findings_sampled =
            self.samples.len() as u64 == self.findings().min(MAX_DECODE_CENSUS_SAMPLES as u64);
        if !primaries_sorted
            || self.primaries.iter().any(|row| row.primary > 63)
            || !extended_sorted
            || !extended_under_their_primaries
            || !extended_tile_their_primaries
            || !samples_in_domain
            || !samples_sorted
            || !findings_sampled
            || self.samples.len() > MAX_DECODE_CENSUS_SAMPLES
        {
            return Err(DecodeCensusError::InvalidArtifact);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/decode_census_tests.rs"]
mod tests;
