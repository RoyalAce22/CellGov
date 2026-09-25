//! Streaming per-step state-hash divergence scanner.
//!
//! Walks two binary trace streams, filters each to its `PpuStateHash`
//! records, and reports the first index where they disagree. Scan is
//! O(min(len_a, len_b)) with constant auxiliary memory: both streams
//! are consumed as iterators and never materialized.
//!
//! [Armstrong2019 p:71:24 s:7] A trace comparison between two
//! simulators checks that they execute matching instructions and make
//! matching register writes; here the PC is checked before the hash.
//!
//! `PpuStateHash` covers scalar integer state only, so the reported step
//! is the first *scalar-visible* disagreement. Two runs diverging in a
//! float or vector register agree here until that value reaches a covered
//! register, which can be arbitrarily far downstream.

use cellgov_trace::{TraceReader, TraceRecord};

use crate::trace_decode::TraceDecodeError;

/// Which field disagreed at the first differing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergeField {
    /// PCs differ at this step.
    Pc,
    /// PCs match but state hashes differ at this step.
    Hash,
}

/// Outcome of comparing two per-step state-hash streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergeReport {
    /// The two streams hold PPU hashes of two schemes; see [`trace_scheme`].
    /// The scan compares no record and claims no divergence.
    SchemeMismatch {
        /// Scheme id of side A.
        a: u64,
        /// Scheme id of side B.
        b: u64,
    },
    /// All `count` records matched pairwise and both streams ended.
    Identical {
        /// Records matched on each side.
        count: u64,
    },
    /// Both sides reached `step` but disagreed on `field`.
    Differs {
        /// 0-based index of the first scalar-visible disagreement; see the
        /// module docs for why that is not always the first divergence.
        step: u64,
        /// PC on side A.
        a_pc: u64,
        /// PC on side B.
        b_pc: u64,
        /// State hash on side A.
        a_hash: u64,
        /// State hash on side B.
        b_hash: u64,
        /// Which field broke first.
        field: DivergeField,
    },
    /// One side ended before the other; `common_count` records matched.
    LengthDiffers {
        /// Records matched before either side ended.
        common_count: u64,
        /// Total `PpuStateHash` records in side A.
        a_count: u64,
        /// Total `PpuStateHash` records in side B.
        b_count: u64,
    },
    /// A trace stopped decoding before the scan finished. Not a verdict
    /// on the runs: the `common_count` records before the failure
    /// matched, and nothing past it was compared.
    CorruptTrace {
        /// Records matched before the failure.
        common_count: u64,
        /// Decode failure on side A, if that side failed.
        a_error: Option<TraceDecodeError>,
        /// Decode failure on side B, if that side failed.
        b_error: Option<TraceDecodeError>,
    },
}

/// The scheme ids a trace stream names for its state hashes.
///
/// A tool that compares one record kind across two streams checks that
/// kind's id first, and reports a scheme mismatch when the ids differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceSchemes {
    /// Scheme id of the stream's `PpuStateHash` records.
    pub ppu: u64,
    /// Scheme id of the stream's `StateHashCheckpoint` records.
    pub checkpoint: u64,
}

impl TraceSchemes {
    /// The schemes of a stream that has no `StateHashScheme` record.
    pub const UNSTAMPED: Self = Self {
        ppu: cellgov_ppu::state::FNV1A_SCHEME_ID,
        checkpoint: crate::observation::LEGACY_CHECKPOINT_HASH_SCHEME,
    };
}

/// The scheme ids of a stream's state hashes.
///
/// The ids come from the `StateHashScheme` record after the header. A
/// stream without that record reads as [`TraceSchemes::UNSTAMPED`]. A
/// stream whose leading records do not decode reads the same way;
/// [`diverge`] reports the decode failure for that stream.
pub fn trace_scheme(bytes: &[u8]) -> TraceSchemes {
    leading_scheme(bytes).unwrap_or(TraceSchemes::UNSTAMPED)
}

/// The scheme ids of a stream, or `None` when a leading record does not
/// decode and so the schemes are unknown.
fn leading_scheme(bytes: &[u8]) -> Option<TraceSchemes> {
    let mut reader = TraceReader::new(bytes);
    let mut first = reader.next();
    if matches!(first, Some(Ok(TraceRecord::RunIdentity { .. }))) {
        first = reader.next();
    }
    match first {
        Some(Err(_)) => None,
        Some(Ok(TraceRecord::StateHashScheme { ppu, checkpoint })) => {
            Some(TraceSchemes { ppu, checkpoint })
        }
        _ => Some(TraceSchemes::UNSTAMPED),
    }
}

/// Walk two trace byte slices and report the first `PpuStateHash` divergence.
///
/// The scan ends early in two cases:
///
/// - The two streams name two PPU schemes. The scan returns
///   [`DivergeReport::SchemeMismatch`] and reads no `PpuStateHash`. The
///   checkpoint schemes do not stop the scan, since it reads no
///   checkpoint record.
/// - A record on either side fails to decode. The scan returns
///   [`DivergeReport::CorruptTrace`], even when the record lies past the
///   other side's clean end.
///
/// A side whose leading records do not decode has no known scheme, so
/// the scan reports the decode failure. That failure comes before any
/// `PpuStateHash`, so the scan compares no hash.
pub fn diverge(a: &[u8], b: &[u8]) -> DivergeReport {
    if let (Some(a_scheme), Some(b_scheme)) = (leading_scheme(a), leading_scheme(b)) {
        if a_scheme.ppu != b_scheme.ppu {
            return DivergeReport::SchemeMismatch {
                a: a_scheme.ppu,
                b: b_scheme.ppu,
            };
        }
    }
    let mut ai = state_hash_iter(a);
    let mut bi = state_hash_iter(b);
    let mut step: u64 = 0;
    loop {
        let (a_next, b_next) = match (ai.next().transpose(), bi.next().transpose()) {
            (Ok(a_next), Ok(b_next)) => (a_next, b_next),
            (a_next, b_next) => {
                return DivergeReport::CorruptTrace {
                    common_count: step,
                    a_error: a_next.err(),
                    b_error: b_next.err(),
                }
            }
        };
        match (a_next, b_next) {
            (None, None) => return DivergeReport::Identical { count: step },
            (Some(_), None) => {
                return match remaining(ai) {
                    Ok(rest) => DivergeReport::LengthDiffers {
                        common_count: step,
                        a_count: step + 1 + rest,
                        b_count: step,
                    },
                    Err(error) => DivergeReport::CorruptTrace {
                        common_count: step,
                        a_error: Some(error),
                        b_error: None,
                    },
                };
            }
            (None, Some(_)) => {
                return match remaining(bi) {
                    Ok(rest) => DivergeReport::LengthDiffers {
                        common_count: step,
                        a_count: step,
                        b_count: step + 1 + rest,
                    },
                    Err(error) => DivergeReport::CorruptTrace {
                        common_count: step,
                        a_error: None,
                        b_error: Some(error),
                    },
                };
            }
            (Some((a_pc, a_hash)), Some((b_pc, b_hash))) => {
                if a_pc != b_pc {
                    return DivergeReport::Differs {
                        step,
                        a_pc,
                        b_pc,
                        a_hash,
                        b_hash,
                        field: DivergeField::Pc,
                    };
                }
                if a_hash != b_hash {
                    return DivergeReport::Differs {
                        step,
                        a_pc,
                        b_pc,
                        a_hash,
                        b_hash,
                        field: DivergeField::Hash,
                    };
                }
                step += 1;
            }
        }
    }
}

/// Count the `PpuStateHash` records left on one side after the other
/// ended, failing at the first record that does not decode.
fn remaining(
    iter: impl Iterator<Item = Result<(u64, u64), TraceDecodeError>>,
) -> Result<u64, TraceDecodeError> {
    let mut count = 0;
    for record in iter {
        record?;
        count += 1;
    }
    Ok(count)
}

/// Iterate `PpuStateHash` records as `(pc, hash)`, skipping other record
/// kinds; a decode failure is yielded once, positioned by the index and
/// byte offset of the record that failed, and then the stream ends.
fn state_hash_iter(
    bytes: &[u8],
) -> impl Iterator<Item = Result<(u64, u64), TraceDecodeError>> + '_ {
    let mut reader = TraceReader::new(bytes);
    let mut index = 0usize;
    std::iter::from_fn(move || loop {
        let offset = reader.position();
        match reader.next()? {
            Ok(TraceRecord::PpuStateHash { pc, hash, .. }) => {
                index += 1;
                return Some(Ok((pc, hash.raw())));
            }
            Ok(_) => index += 1,
            Err(source) => {
                return Some(Err(TraceDecodeError {
                    index,
                    offset,
                    source,
                }))
            }
        }
    })
}

#[cfg(test)]
#[path = "tests/scan_tests.rs"]
mod tests;
