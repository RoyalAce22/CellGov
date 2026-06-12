//! Streaming per-step state-hash divergence scanner.
//!
//! Walks two binary trace streams, filters each to its `PpuStateHash`
//! records, and reports the first index where they disagree. Scan is
//! O(min(len_a, len_b)) with constant auxiliary memory: both streams
//! are consumed as iterators and never materialized.

use cellgov_trace::{TraceReader, TraceRecord};

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
    /// All `count` records matched pairwise and both streams ended.
    Identical {
        /// Records matched on each side.
        count: u64,
    },
    /// Both sides reached `step` but disagreed on `field`.
    Differs {
        /// 0-based step index where the disagreement occurred.
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
}

/// Walk two trace byte slices and report the first `PpuStateHash` divergence.
///
/// Decode errors truncate the affected iterator, which surfaces as a
/// `LengthDiffers` result; callers needing to distinguish "malformed"
/// from "ended" must validate the inputs separately.
pub fn diverge(a: &[u8], b: &[u8]) -> DivergeReport {
    let mut ai = state_hash_iter(a);
    let mut bi = state_hash_iter(b);
    let mut step: u64 = 0;
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return DivergeReport::Identical { count: step },
            (Some(_), None) => {
                let a_count = step + 1 + ai.count() as u64;
                return DivergeReport::LengthDiffers {
                    common_count: step,
                    a_count,
                    b_count: step,
                };
            }
            (None, Some(_)) => {
                let b_count = step + 1 + bi.count() as u64;
                return DivergeReport::LengthDiffers {
                    common_count: step,
                    a_count: step,
                    b_count,
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

/// Iterate `PpuStateHash` records as `(pc, hash)`; other kinds and decode errors are skipped.
fn state_hash_iter(bytes: &[u8]) -> impl Iterator<Item = (u64, u64)> + '_ {
    TraceReader::new(bytes).filter_map(|r| match r {
        Ok(TraceRecord::PpuStateHash { pc, hash, .. }) => Some((pc, hash.raw())),
        _ => None,
    })
}

#[cfg(test)]
#[path = "tests/scan_tests.rs"]
mod tests;
