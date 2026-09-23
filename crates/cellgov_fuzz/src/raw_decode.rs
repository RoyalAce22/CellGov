//! Bounded raw decoder scans with replayable, versioned results.

use std::convert::Infallible;

use serde::{Deserialize, Serialize};

use crate::sweep::{sweep_finite, DecodePanic, FiniteCase, FiniteSweepError, FiniteVerdict};

/// Version of the normalized raw-word sweep artifact.
pub const RAW_DECODE_SCHEMA_VERSION: u32 = 1;
/// Largest chunk retained by one finite-domain sweep.
pub const MAX_RAW_DECODE_CHUNK: usize = 1 << 16;
/// Maximum detailed panic samples retained in one result.
pub const MAX_RAW_DECODE_PANIC_SAMPLES: usize = 128;

/// Interpreter decoder whose identity a replay must preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawDecoder {
    /// PowerPC decoder.
    Ppu,
    /// Synergistic-processor decoder.
    Spu,
}

/// Inclusive-start, counted interval in the 32-bit instruction-word space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDecodeDomain {
    /// First instruction word.
    pub first: u32,
    /// Number of consecutive words, including the first.
    pub count: u64,
}

/// Typed refusal to construct or finish a raw decoder sweep.
#[derive(Debug, thiserror::Error)]
pub enum RawDecodeError {
    /// Artifact JSON could not be decoded.
    #[error("raw decoder artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Artifact uses an unsupported schema version.
    #[error("raw decoder schema version {found} is unsupported; expected {supported}")]
    Version {
        /// Artifact version.
        found: u32,
        /// Supported version.
        supported: u32,
    },
    /// Counts, completion status, or panic samples contradict the word domain.
    #[error("raw decoder artifact has inconsistent counts or panic samples")]
    InvalidArtifact,
    /// Empty or overflowing domain.
    #[error("raw decoder domain starting at 0x{first:08x} cannot hold {count} words")]
    Domain {
        /// First word.
        first: u32,
        /// Requested count.
        count: u64,
    },
    /// A full-domain shard index does not belong to its partition count.
    #[error("raw decoder shard {index} is invalid for {shards} shards")]
    Shard {
        /// Requested shard index.
        index: u32,
        /// Shard count.
        shards: u32,
    },
    /// Chunk bound is empty or exceeds the memory-limited maximum.
    #[error("raw decoder chunk size {requested} must be within 1..={MAX_RAW_DECODE_CHUNK}")]
    Chunk {
        /// Requested chunk length.
        requested: usize,
    },
    /// Cancellation offset exceeds the selected domain.
    #[error("raw decoder cancellation offset {offset} exceeds {count} words")]
    Cancellation {
        /// Requested offset.
        offset: u64,
        /// Selected domain size.
        count: u64,
    },
    /// Finite worker or sink failed.
    #[error("raw decoder finite sweep failed: {0}")]
    Sweep(#[from] FiniteSweepError<Infallible, Infallible>),
}

impl RawDecodeError {
    /// Distinguishes invalid caller settings from execution or artifact failures.
    pub const fn is_invalid_request(&self) -> bool {
        match self {
            Self::Domain { .. }
            | Self::Shard { .. }
            | Self::Chunk { .. }
            | Self::Cancellation { .. }
            | Self::Sweep(FiniteSweepError::ZeroWorkers) => true,
            Self::Json(_) | Self::Version { .. } | Self::InvalidArtifact | Self::Sweep(_) => false,
        }
    }
}

impl RawDecodeDomain {
    /// Returns the final word only when the interval fits the 32-bit space.
    pub fn word_at_last(self) -> Option<u32> {
        self.count
            .checked_sub(1)
            .and_then(|offset| u64::from(self.first).checked_add(offset))
            .and_then(|last| u32::try_from(last).ok())
    }
    /// Constructs a bounded interval without wrapping past the final word.
    ///
    /// # Errors
    ///
    /// Refuses an empty interval or an end beyond the 32-bit word space.
    pub fn new(first: u32, count: u64) -> Result<Self, RawDecodeError> {
        if count == 0
            || u64::from(first)
                .checked_add(count - 1)
                .is_none_or(|last| last > u64::from(u32::MAX))
        {
            return Err(RawDecodeError::Domain { first, count });
        }
        Ok(Self { first, count })
    }

    /// Selects one stable shard of the entire 32-bit word space.
    ///
    /// # Errors
    ///
    /// Refuses zero shards or an index outside the shard count.
    pub fn full_shard(index: u32, shards: u32) -> Result<Self, RawDecodeError> {
        if shards == 0 || index >= shards {
            return Err(RawDecodeError::Shard { index, shards });
        }
        let full = u64::from(u32::MAX) + 1;
        let base = full / u64::from(shards);
        let extra = full % u64::from(shards);
        let first = base * u64::from(index) + extra.min(u64::from(index));
        let count = base + u64::from(index < extra as u32);
        let first = u32::try_from(first).map_err(|_| RawDecodeError::Shard { index, shards })?;
        Self::new(first, count)
    }
}

/// Completion class carried in the offline artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawDecodeStatus {
    /// Every selected word was classified.
    Complete,
    /// The caller stopped at a deterministic prefix boundary.
    Cancelled,
}

/// Normalized machine-readable result of a bounded or full decoder scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDecodeArtifact {
    /// Artifact schema version.
    pub schema_version: u32,
    /// Decoder used to classify the raw words.
    pub decoder: RawDecoder,
    /// Selected word interval.
    pub domain: RawDecodeDomain,
    /// Completion status, including explicit cancellation.
    pub status: RawDecodeStatus,
    /// Number of words classified before completion or cancellation.
    pub processed: u64,
    /// Successfully decoded words.
    pub accepted: u64,
    /// Decoder refusals.
    pub refused: u64,
    /// Decoder panics, including those not retained in the bounded sample.
    pub panics: u64,
    /// First bounded set of raw words and typed panic payloads.
    pub panic_samples: Vec<DecodePanic>,
}

impl RawDecodeArtifact {
    /// Parses a versioned result and validates its replay coordinates.
    ///
    /// # Errors
    ///
    /// Refuses malformed JSON, unsupported versions, or inconsistent counts.
    pub fn parse_json(json: &str) -> Result<Self, RawDecodeError> {
        let artifact: Self = serde_json::from_str(json)?;
        if artifact.schema_version != RAW_DECODE_SCHEMA_VERSION {
            return Err(RawDecodeError::Version {
                found: artifact.schema_version,
                supported: RAW_DECODE_SCHEMA_VERSION,
            });
        }
        RawDecodeDomain::new(artifact.domain.first, artifact.domain.count)?;
        let covered = artifact
            .accepted
            .checked_add(artifact.refused)
            .and_then(|count| count.checked_add(artifact.panics));
        let valid_status = match artifact.status {
            RawDecodeStatus::Complete => artifact.processed == artifact.domain.count,
            RawDecodeStatus::Cancelled => artifact.processed < artifact.domain.count,
        };
        let valid_samples = artifact.panic_samples.len() as u64
            == artifact.panics.min(MAX_RAW_DECODE_PANIC_SAMPLES as u64)
            && artifact
                .panic_samples
                .windows(2)
                .all(|pair| pair[0].raw < pair[1].raw)
            && artifact.panic_samples.iter().all(|sample| {
                u64::from(sample.raw)
                    .checked_sub(u64::from(artifact.domain.first))
                    .is_some_and(|offset| offset < artifact.processed)
            });
        if !valid_status || covered != Some(artifact.processed) || !valid_samples {
            return Err(RawDecodeError::InvalidArtifact);
        }
        Ok(artifact)
    }
    /// Reports whether the entire interval finished without a target panic.
    pub fn is_clean(&self) -> bool {
        self.status == RawDecodeStatus::Complete
            && self.panics == 0
            && self.processed == self.domain.count
            && self.accepted.checked_add(self.refused) == Some(self.domain.count)
    }

    /// Returns the raw word at one offset, for deterministic replay.
    pub fn word_at(&self, offset: u64) -> Option<u32> {
        (offset < self.domain.count)
            .then(|| u64::from(self.domain.first).checked_add(offset))
            .flatten()
            .and_then(|value| u32::try_from(value).ok())
    }
}

/// Scans bounded chunks without allocating the entire 32-bit word space.
///
/// # Errors
///
/// Refuses invalid bounds, invalid worker settings, or a finite sweep failure.
pub fn scan_raw_decoder(
    decoder: RawDecoder,
    domain: RawDecodeDomain,
    chunk_size: usize,
    workers: usize,
    cancel_after: Option<u64>,
) -> Result<RawDecodeArtifact, RawDecodeError> {
    RawDecodeDomain::new(domain.first, domain.count)?;
    if chunk_size == 0 || chunk_size > MAX_RAW_DECODE_CHUNK {
        return Err(RawDecodeError::Chunk {
            requested: chunk_size,
        });
    }
    if cancel_after.is_some_and(|offset| offset > domain.count) {
        return Err(RawDecodeError::Cancellation {
            offset: cancel_after.unwrap_or(0),
            count: domain.count,
        });
    }
    let limit = cancel_after.unwrap_or(domain.count);
    let mut artifact = RawDecodeArtifact {
        schema_version: RAW_DECODE_SCHEMA_VERSION,
        decoder,
        domain,
        status: if limit == domain.count {
            RawDecodeStatus::Complete
        } else {
            RawDecodeStatus::Cancelled
        },
        processed: 0,
        accepted: 0,
        refused: 0,
        panics: 0,
        panic_samples: Vec::new(),
    };
    if workers == 0 {
        return Err(RawDecodeError::Sweep(FiniteSweepError::ZeroWorkers));
    }
    while artifact.processed < limit {
        let remaining = limit - artifact.processed;
        let batch = remaining.min(chunk_size as u64) as usize;
        let start = u64::from(domain.first) + artifact.processed;
        let words = (0..batch)
            .map(|offset| (start + offset as u64) as u32)
            .collect::<Vec<_>>();
        let result = sweep_finite(
            &words,
            workers,
            None,
            |&raw| {
                let accepted = match decoder {
                    RawDecoder::Ppu => cellgov_ppu::decode::decode(raw).is_ok(),
                    RawDecoder::Spu => cellgov_spu::decode::decode(raw).is_ok(),
                };
                Ok::<_, Infallible>(if accepted {
                    FiniteVerdict::Accepted(())
                } else {
                    FiniteVerdict::Refused
                })
            },
            |case| {
                if let FiniteCase::Panicked { index, payload } = case {
                    if artifact.panic_samples.len() < MAX_RAW_DECODE_PANIC_SAMPLES {
                        artifact.panic_samples.push(DecodePanic {
                            raw: words[*index],
                            payload: payload.clone(),
                        });
                    }
                }
                Ok::<_, Infallible>(())
            },
        )?;
        artifact.accepted += result.accepted;
        artifact.refused += result.refused;
        artifact.panics += result.panics;
        artifact.processed += result.total;
    }
    Ok(artifact)
}

#[cfg(test)]
#[path = "tests/raw_decode_tests.rs"]
mod tests;
